"""Minimal ACP client used by the Hugging Face voice UI.

The facade owns persistent stdio processes and ACP sessions. Gemma only sees a
single provider-neutral function tool and never carries agent-harness state.
"""
from __future__ import annotations

import asyncio
import copy
import json
import os
import re
from pathlib import Path
from typing import Any


PROVIDER_ID = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$")


def _content_text(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "".join(_content_text(item) for item in value)
    if isinstance(value, dict):
        if value.get("type") == "text":
            return str(value.get("text", ""))
        return "".join(_content_text(item) for item in value.values())
    return ""


class ACPError(RuntimeError):
    pass


class ACPAgent:
    def __init__(self, name: str, config: dict[str, Any], base_dir: Path):
        self.name = name
        self.config = copy.deepcopy(config)
        cwd = Path(self.config.get("cwd", ".")).expanduser()
        self.cwd = (base_dir / cwd).resolve() if not cwd.is_absolute() else cwd.resolve()
        self.timeout = float(self.config.get("timeoutSeconds", 180))
        self.process: asyncio.subprocess.Process | None = None
        self.session_id: str | None = None
        self.request_id = 0
        self.lock = asyncio.Lock()
        self.stderr_tail: list[str] = []
        self._stderr_task: asyncio.Task | None = None

    async def _start(self) -> None:
        if self.process and self.process.returncode is None:
            return
        self.session_id = None
        env = dict(os.environ)
        env.update({str(key): str(value) for key, value in self.config.get("env", {}).items()})
        self.process = await asyncio.create_subprocess_exec(
            self.config["command"],
            *self.config.get("args", []),
            cwd=str(self.cwd),
            env=env,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        self._stderr_task = asyncio.create_task(self._drain_stderr())

    async def _drain_stderr(self) -> None:
        assert self.process and self.process.stderr
        while line := await self.process.stderr.readline():
            self.stderr_tail.append(line.decode(errors="replace").rstrip())
            del self.stderr_tail[:-20]

    async def _write(self, message: dict[str, Any]) -> None:
        assert self.process and self.process.stdin
        self.process.stdin.write(json.dumps(message, separators=(",", ":")).encode() + b"\n")
        await self.process.stdin.drain()

    async def _request(self, method: str, params: dict[str, Any], chunks: list[str] | None = None) -> dict:
        await self._start()
        assert self.process and self.process.stdout
        self.request_id += 1
        request_id = self.request_id
        await self._write({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})

        async def receive() -> dict:
            while True:
                line = await self.process.stdout.readline()
                if not line:
                    detail = " ".join(self.stderr_tail)[-800:]
                    raise ACPError(f"{self.name} exited before replying{': ' + detail if detail else ''}")
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if message.get("id") == request_id:
                    if message.get("error"):
                        raise ACPError(message["error"].get("message", str(message["error"])))
                    return message.get("result") or {}
                if message.get("method") == "session/update" and chunks is not None:
                    update = (message.get("params") or {}).get("update") or {}
                    if update.get("sessionUpdate") == "agent_message_chunk":
                        text = _content_text(update.get("content"))
                        if text:
                            chunks.append(text)
                elif "id" in message and "method" in message:
                    # Interactive approval is deliberately outside this first facade.
                    await self._write({"jsonrpc": "2.0", "id": message["id"],
                                       "result": {"outcome": {"outcome": "cancelled"}}})

        try:
            if self.timeout > 0:
                return await asyncio.wait_for(receive(), timeout=self.timeout)
            return await receive()
        except asyncio.TimeoutError as exc:
            raise ACPError(f"{self.name} did not finish within {self.timeout:g} seconds") from exc

    async def _initialize(self) -> None:
        await self._start()
        if self.session_id:
            return
        await self._request("initialize", {
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {"name": "open-voice", "title": "Open Voice", "version": "0.1.0"},
        })
        session = await self._request("session/new", {"cwd": str(self.cwd), "mcpServers": []})
        self.session_id = session.get("sessionId")
        if not self.session_id:
            raise ACPError(f"{self.name} did not return an ACP session ID")

    async def prompt(self, text: str) -> str:
        async with self.lock:
            await self._initialize()
            chunks: list[str] = []
            await self._request("session/prompt", {
                "sessionId": self.session_id,
                "prompt": [{"type": "text", "text": text}],
            }, chunks)
            return "".join(chunks).strip() or "The agent completed the task without a text response."

    async def close(self) -> None:
        if self.process and self.process.returncode is None:
            self.process.terminate()
            try:
                await asyncio.wait_for(self.process.wait(), timeout=2)
            except asyncio.TimeoutError:
                self.process.kill()
                await self.process.wait()
        if self._stderr_task:
            self._stderr_task.cancel()


class ACPFacade:
    def __init__(self, config_path: Path):
        self.config_path = config_path.resolve()
        self.config_lock = asyncio.Lock()
        self.default_agent = ""
        self.agents: dict[str, ACPAgent] = {}
        self._mtime_ns = 0
        self._load_initial_config()

    @staticmethod
    def _validate_provider(name: str, definition: dict[str, Any]) -> dict[str, Any]:
        if not PROVIDER_ID.fullmatch(name):
            raise ValueError("Provider ID must use 1-64 letters, numbers, dots, dashes, or underscores")
        command = str(definition.get("command", "")).strip()
        if not command:
            raise ValueError("ACP provider command must not be empty")
        args = definition.get("args", [])
        env = definition.get("env", {})
        aliases = definition.get("aliases", [])
        if not isinstance(args, list) or any(not isinstance(item, str) for item in args):
            raise ValueError("ACP provider args must be a list of strings")
        if not isinstance(env, dict) or any(not isinstance(key, str) or not isinstance(value, str)
                                            for key, value in env.items()):
            raise ValueError("ACP provider env must map names to string values")
        if not isinstance(aliases, list) or any(not isinstance(item, str) or not item.strip()
                                                for item in aliases):
            raise ValueError("ACP provider aliases must be a list of non-empty strings")
        timeout = float(definition.get("timeoutSeconds", 180))
        if timeout < 0 or (0 < timeout < 1) or timeout > 3600:
            raise ValueError("ACP provider timeout must be 0 (no timeout) or between 1 and 3600 seconds")
        return {
            "displayName": str(definition.get("displayName", name)).strip() or name,
            "aliases": list(dict.fromkeys(item.strip() for item in aliases)),
            "guidance": str(definition.get("guidance", "")).strip(),
            "command": command,
            "args": args,
            "cwd": str(definition.get("cwd", ".")).strip() or ".",
            "timeoutSeconds": timeout,
            "env": env,
        }

    @staticmethod
    def _validate_names(definitions: dict[str, dict[str, Any]]) -> None:
        claimed: dict[str, str] = {}
        for provider_id, definition in definitions.items():
            names = [provider_id, definition["displayName"], *definition["aliases"]]
            for value in names:
                key = value.casefold()
                owner = claimed.get(key)
                if owner and owner != provider_id:
                    raise ValueError(f"ACP provider name or alias {value!r} is already used by {owner!r}")
                claimed[key] = provider_id

    def _read_config(self) -> tuple[str, dict[str, dict[str, Any]]]:
        config = json.loads(self.config_path.read_text())
        definitions = config.get("agents") or {}
        if not isinstance(definitions, dict):
            raise ValueError("ACP configuration agents must be an object")
        validated = {
            str(name): self._validate_provider(str(name), definition)
            for name, definition in definitions.items()
            if isinstance(definition, dict)
        }
        self._validate_names(validated)
        default = str(config.get("defaultAgent", "")).strip()
        if default and default not in validated:
            raise ValueError(f"Default ACP provider {default!r} is not configured")
        if not default and validated:
            default = next(iter(validated))
        return default, validated

    def _load_initial_config(self) -> None:
        default, definitions = self._read_config()
        self.default_agent = default
        self.agents = {
            name: ACPAgent(name, definition, self.config_path.parent)
            for name, definition in definitions.items()
        }
        self._mtime_ns = self.config_path.stat().st_mtime_ns

    async def refresh(self) -> None:
        """Reload changes written by the other UI process (HTTP or HTTPS)."""
        try:
            mtime_ns = self.config_path.stat().st_mtime_ns
        except FileNotFoundError:
            return
        if mtime_ns == self._mtime_ns:
            return
        async with self.config_lock:
            if self.config_path.stat().st_mtime_ns == self._mtime_ns:
                return
            default, definitions = self._read_config()
            old_agents = self.agents
            next_agents: dict[str, ACPAgent] = {}
            retired: list[ACPAgent] = []
            for name, definition in definitions.items():
                old = old_agents.get(name)
                if old and old.config == definition:
                    next_agents[name] = old
                else:
                    next_agents[name] = ACPAgent(name, definition, self.config_path.parent)
                    if old:
                        retired.append(old)
            retired.extend(agent for name, agent in old_agents.items() if name not in definitions)
            self.agents = next_agents
            self.default_agent = default
            self._mtime_ns = self.config_path.stat().st_mtime_ns
        await asyncio.gather(*(agent.close() for agent in retired))

    def providers(self) -> dict[str, Any]:
        return {
            "defaultProvider": self.default_agent or None,
            "providers": [
                {
                    "id": name,
                    "displayName": agent.config["displayName"],
                    "aliases": agent.config.get("aliases", []),
                    "guidance": agent.config.get("guidance", ""),
                    "command": agent.config["command"],
                    "args": agent.config.get("args", []),
                    "cwd": agent.config.get("cwd", "."),
                    "timeoutSeconds": agent.config.get("timeoutSeconds", 180),
                    "envKeys": sorted((agent.config.get("env") or {}).keys()),
                }
                for name, agent in self.agents.items()
            ],
        }

    def _write_config(self, default: str, definitions: dict[str, dict[str, Any]]) -> None:
        payload = {"defaultAgent": default, "agents": definitions}
        temporary = self.config_path.with_name(f".{self.config_path.name}.{os.getpid()}.tmp")
        temporary.write_text(json.dumps(payload, indent=2) + "\n")
        # Provider environments may contain gateway tokens or API keys. Keep
        # the on-disk registry private even when the user's umask is permissive.
        temporary.chmod(0o600)
        os.replace(temporary, self.config_path)

    async def upsert_provider(self, name: str, definition: dict[str, Any]) -> dict[str, Any]:
        await self.refresh()
        async with self.config_lock:
            existing = self.agents.get(name)
            if definition.get("env") is None:
                definition["env"] = copy.deepcopy(existing.config.get("env", {})) if existing else {}
            clean = self._validate_provider(name, definition)
            definitions = {key: copy.deepcopy(agent.config) for key, agent in self.agents.items()}
            definitions[name] = clean
            self._validate_names(definitions)
            default = self.default_agent or name
            self._write_config(default, definitions)
        await self.refresh()
        return self.providers()

    async def remove_provider(self, name: str) -> dict[str, Any]:
        await self.refresh()
        async with self.config_lock:
            if name not in self.agents:
                raise ValueError(f"Unknown ACP provider {name!r}")
            definitions = {
                key: copy.deepcopy(agent.config)
                for key, agent in self.agents.items()
                if key != name
            }
            default = self.default_agent if self.default_agent != name else next(iter(definitions), "")
            self._write_config(default, definitions)
        await self.refresh()
        return self.providers()

    async def set_default(self, name: str) -> dict[str, Any]:
        await self.refresh()
        async with self.config_lock:
            if name not in self.agents:
                raise ValueError(f"Unknown ACP provider {name!r}")
            definitions = {key: copy.deepcopy(agent.config) for key, agent in self.agents.items()}
            self._write_config(name, definitions)
        await self.refresh()
        return self.providers()

    async def delegate(self, task: str, agent: str | None = None) -> dict[str, str]:
        await self.refresh()
        requested = (agent or self.default_agent).strip()
        target = self.resolve_provider(requested) if requested else ""
        if not target:
            raise ValueError("No ACP provider is configured")
        if target not in self.agents:
            raise ValueError(f"Unknown ACP agent {requested!r}; available: {', '.join(self.agents)}")
        clean_task = task.strip()
        if not clean_task:
            raise ValueError("task must not be empty")
        result = await self.agents[target].prompt(clean_task)
        return {"agent": target, "name": self.agents[target].config["displayName"], "result": result}

    def resolve_provider(self, requested: str) -> str:
        key = requested.strip().casefold()
        for provider_id, provider in self.agents.items():
            names = [provider_id, provider.config["displayName"], *provider.config.get("aliases", [])]
            if any(key == name.casefold() for name in names):
                return provider_id
        return requested.strip()

    def routing_guide(self) -> str:
        if not self.agents:
            return "No ACP providers are configured."
        lines = []
        for provider_id, provider in self.agents.items():
            config = provider.config
            names = [config["displayName"], *config.get("aliases", [])]
            guidance = config.get("guidance") or "General delegated agent work."
            lines.append(f"{provider_id}: names {', '.join(names)}; {guidance}")
        return "Available ACP providers: " + " | ".join(lines)

    async def close(self) -> None:
        await asyncio.gather(*(agent.close() for agent in self.agents.values()))
