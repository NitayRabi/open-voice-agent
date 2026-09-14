"""Same-origin ACP facade layered over the unmodified HF demo app."""
from __future__ import annotations

import asyncio
import importlib.util
import json
import os
import sys
from pathlib import Path

import websockets
from fastapi import FastAPI, HTTPException, Request, WebSocket
from fastapi.responses import HTMLResponse, Response
from pydantic import BaseModel, Field

from acp_facade import ACPError, ACPFacade
from delegation_manager import DelegationManager

HERE = Path(__file__).resolve().parent
UPSTREAM = Path(os.environ["HF_S2S_UPSTREAM"]).resolve()
DEMO = UPSTREAM / "demo"
sys.path.insert(0, str(DEMO))
spec = importlib.util.spec_from_file_location("hf_s2s_demo_server", DEMO / "server.py")
assert spec and spec.loader
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

facade = ACPFacade(Path(os.environ.get("OPEN_VOICE_AGENTS", HERE / "agents.json")))
delegations = DelegationManager(facade)
app = FastAPI(title="Open Voice")
REALTIME_UPSTREAM = os.environ.get(
    "OPEN_VOICE_REALTIME_UPSTREAM", "ws://127.0.0.1:8766/v1/realtime"
)


class DelegateRequest(BaseModel):
    task: str
    agent: str | None = None


class ProviderRequest(BaseModel):
    displayName: str = ""
    aliases: list[str] = Field(default_factory=list)
    guidance: str = ""
    command: str
    args: list[str] = Field(default_factory=list)
    cwd: str = "."
    timeoutSeconds: float = 180
    env: dict[str, str] | None = None


class DefaultProviderRequest(BaseModel):
    provider: str


def require_local(request: Request) -> None:
    """ACP command execution is remote-accessible only through the paired node."""
    host = request.client.host if request.client else ""
    if host not in {"127.0.0.1", "::1"}:
        raise HTTPException(
            status_code=403,
            detail="ACP delegation is available remotely through the paired Open Voice node",
        )


@app.post("/api/delegate")
async def delegate(request: DelegateRequest, http_request: Request):
    require_local(http_request)
    try:
        return await facade.delegate(request.task, request.agent)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    except ACPError as exc:
        raise HTTPException(status_code=502, detail=str(exc)) from exc


@app.post("/api/delegations", status_code=202)
async def start_delegation(request: DelegateRequest, http_request: Request):
    require_local(http_request)
    try:
        return await delegations.start(request.task, request.agent)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.get("/api/delegations")
async def list_delegations(http_request: Request):
    require_local(http_request)
    return {"delegations": delegations.list()}


@app.get("/api/delegations/{job_id}")
async def get_delegation(job_id: str, http_request: Request):
    require_local(http_request)
    try:
        return delegations.get(job_id)
    except ValueError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc


@app.websocket("/v1/realtime")
async def realtime_proxy(client: WebSocket):
    """Same-origin WebSocket bridge for browsers and desktop shells.

    A reverse proxy may terminate HTTPS/WSS while the working Hugging Face
    realtime service remains private on localhost.
    """
    try:
        offered_subprotocols = [
            value.strip()
            for value in client.headers.get("sec-websocket-protocol", "").split(",")
            if value.strip()
        ]
        async with websockets.connect(
            REALTIME_UPSTREAM,
            max_size=None,
            subprotocols=offered_subprotocols or None,
        ) as upstream:
            await client.accept(subprotocol=upstream.subprotocol)

            async def client_to_upstream() -> None:
                while True:
                    message = await client.receive()
                    if message["type"] == "websocket.disconnect":
                        return
                    if message.get("text") is not None:
                        await upstream.send(message["text"])
                    elif message.get("bytes") is not None:
                        await upstream.send(message["bytes"])

            async def upstream_to_client() -> None:
                async for message in upstream:
                    if isinstance(message, str):
                        await client.send_text(message)
                    else:
                        await client.send_bytes(message)

            tasks = {
                asyncio.create_task(client_to_upstream()),
                asyncio.create_task(upstream_to_client()),
            }
            done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for task in pending:
                task.cancel()
            await asyncio.gather(*pending, return_exceptions=True)
            for task in done:
                task.result()
    except Exception:
        # Either side may already have closed; close is intentionally best-effort.
        try:
            if client.client_state.name == "CONNECTING":
                await client.accept()
            await client.close(code=1011)
        except RuntimeError:
            pass


@app.get("/api/agents")
async def agents(http_request: Request):
    require_local(http_request)
    await facade.refresh()
    return {"defaultAgent": facade.default_agent, "agents": list(facade.agents)}


@app.get("/api/acp/providers")
async def acp_providers(http_request: Request):
    require_local(http_request)
    await facade.refresh()
    return facade.providers()


@app.put("/api/acp/providers/{provider_id}")
async def save_acp_provider(provider_id: str, request: ProviderRequest, http_request: Request):
    require_local(http_request)
    try:
        return await facade.upsert_provider(provider_id, request.model_dump())
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.delete("/api/acp/providers/{provider_id}")
async def delete_acp_provider(provider_id: str, http_request: Request):
    require_local(http_request)
    try:
        return await facade.remove_provider(provider_id)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.put("/api/acp/default")
async def set_default_acp_provider(request: DefaultProviderRequest, http_request: Request):
    require_local(http_request)
    try:
        return await facade.set_default(request.provider)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.post("/api/acp/providers/{provider_id}/test")
async def test_acp_provider(provider_id: str, http_request: Request):
    require_local(http_request)
    try:
        return await facade.delegate("Reply exactly: ACP connection successful.", provider_id)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    except ACPError as exc:
        raise HTTPException(status_code=502, detail=str(exc)) from exc


ACP_SETTINGS_HTML = """
<div class="acp-settings" id="acp-settings">
  <div class="acp-heading">
    <div><strong>ACP providers</strong><small>Delegate agentic work to a local ACP process.</small></div>
    <button id="acp-add" type="button" class="btn ghost">Add provider</button>
  </div>
  <label class="field">
    <span>Default ACP provider</span>
    <select id="acp-default"><option value="">No provider configured</option></select>
  </label>
  <div id="acp-provider-list" class="acp-provider-list"></div>
  <div id="acp-editor" class="acp-editor" hidden>
    <div class="field-row">
      <label class="field"><span>Provider ID</span><input id="acp-id" placeholder="my-agent" /></label>
      <label class="field"><span>Name</span><input id="acp-name" placeholder="Milo" /></label>
    </div>
    <label class="field"><span>Aliases (comma separated)</span><input id="acp-aliases" placeholder="openclaw, claw" /></label>
    <label class="field"><span>Agent guidance</span><textarea id="acp-guidance" rows="3" placeholder="Use this agent for personal automation, messaging, and OpenClaw tasks."></textarea><small>This is shown to Gemma so it knows when and how to route work here.</small></label>
    <label class="field"><span>Command</span><input id="acp-command" placeholder="npx" autocomplete="off" /></label>
    <label class="field"><span>Arguments (one per line)</span><textarea id="acp-args" rows="3" placeholder="-y&#10;@agentclientprotocol/codex-acp"></textarea></label>
    <div class="field-row">
      <label class="field"><span>Working directory</span><input id="acp-cwd" placeholder=".." /></label>
      <label class="field"><span>Timeout (seconds)</span><input id="acp-timeout" type="number" min="1" max="3600" value="180" /></label>
    </div>
    <label class="field"><span>Environment (JSON)</span><textarea id="acp-env" rows="3" placeholder='{"API_KEY":"..."}'></textarea><small id="acp-env-hint">Values are saved locally and never returned to the browser.</small></label>
    <div class="acp-actions">
      <button id="acp-cancel" type="button" class="btn ghost">Cancel</button>
      <button id="acp-save" type="button" class="btn primary">Save provider</button>
    </div>
  </div>
  <small id="acp-status" class="acp-status" aria-live="polite"></small>
</div>
"""

DELEGATION_ACTIVITY_HTML = """
<aside id="delegation-activity" class="delegation-activity" aria-live="polite">
  <header>
    <div><strong>Agent activity</strong><span id="delegation-count" class="delegation-count">Loading…</span></div>
    <button id="delegation-collapse" type="button" aria-label="Collapse delegated work">−</button>
  </header>
  <div id="delegation-list" class="delegation-list"></div>
</aside>
"""


def _inject_acp_settings(source: str) -> str:
    style_marker = '    <link rel="stylesheet" href="style.css" />'
    form_marker = '            <div class="field">\n              <button id="restart-conversation"'
    script_marker = '    <script type="module" src="main.js?v=native-hip-buffered-v3"></script>'
    activity_marker = '    <!-- Ephemeral chat bubbles anchored to the top-right -->'
    changes = [
        (style_marker, style_marker + '\n    <link rel="stylesheet" href="acp-settings.css" />'),
        (form_marker, ACP_SETTINGS_HTML + '\n\n' + form_marker),
        (activity_marker, DELEGATION_ACTIVITY_HTML + '\n\n' + activity_marker),
        (script_marker, script_marker + '\n    <script type="module" src="acp-settings.js"></script>'),
        (script_marker + '\n    <script type="module" src="acp-settings.js"></script>',
         script_marker + '\n    <script type="module" src="acp-settings.js"></script>\n    <script type="module" src="delegation-status.js"></script>'),
        ('main.js?v=native-hip-buffered-v3', 'main.js?v=acp-lifecycle-v2'),
    ]
    for marker, replacement in changes:
        if source.count(marker) != 1:
            raise RuntimeError(f"HF demo settings overlay marker changed: {marker[:60]!r}")
        source = source.replace(marker, replacement)
    return source


@app.get("/", include_in_schema=False)
async def index():
    return HTMLResponse(
        _inject_acp_settings((DEMO / "index.html").read_text()),
        headers={"Cache-Control": "no-store"},
    )


@app.get("/acp-settings.js", include_in_schema=False)
async def acp_settings_js():
    return Response((HERE / "acp-settings.js").read_text(), media_type="text/javascript",
                    headers={"Cache-Control": "no-store"})


@app.get("/acp-settings.css", include_in_schema=False)
async def acp_settings_css():
    return Response((HERE / "acp-settings.css").read_text(), media_type="text/css",
                    headers={"Cache-Control": "no-store"})


@app.get("/delegation-status.js", include_in_schema=False)
async def delegation_status_js():
    return Response((HERE / "delegation-status.js").read_text(), media_type="text/javascript",
                    headers={"Cache-Control": "no-store"})


def _inject_acp_tool(source: str) -> str:
    provider_ids = list(facade.agents)
    provider_schema = json.dumps({
        "type": "string",
        "enum": provider_ids,
        "description": "Provider ID to use. " + facade.routing_guide(),
    }, separators=(",", ":"))
    changes = [
        (
            "const TOOL_DEFS = {\n",
            "const TOOL_DEFS = {\n"
            "  delegate_to_agent: {\n"
            "    type: \"function\",\n"
            "    name: \"delegate_to_agent\",\n"
            "    description: \"MANDATORY action tool. When the user asks, tells, or instructs a named agent/provider to do anything, you MUST call this tool. Never merely say you will ask an agent. Only confirm delegation after this tool returns a real job ID. Also use it for longer, coding, computer, messaging, or agentic work.\",\n"
            "    parameters: { type: \"object\", properties: {\n"
            "      task: { type: \"string\", description: \"A complete, self-contained task for the configured agent.\" },\n"
            f"      agent: {provider_schema},\n"
            "    }, required: [\"task\"] },\n"
            "  },\n",
        ),
        (
            "const defs = [TOOL_DEFS.get_local_time];",
            "const defs = [TOOL_DEFS.get_local_time, TOOL_DEFS.delegate_to_agent];",
        ),
        (
            '    if (name === "get_local_time") {',
            '    if (name === "delegate_to_agent") {\n'
            '      result.output = await execAcpDelegate(String(args.task || ""), String(args.agent || ""));\n'
            '    } else if (name === "get_local_time") {',
        ),
        (
            "/** @param {string} query @returns {Promise<string>} */\nasync function execWebSearch(query) {",
            "/** @param {string} task @param {string} agent @returns {Promise<string>} */\n"
            "async function execAcpDelegate(task, agent = \"\") {\n"
            "  if (!task.trim()) return \"No task provided.\";\n"
            "  const body = agent ? { task, agent } : { task };\n"
            "  const res = await fetch(\"api/delegations\", { method: \"POST\", headers: { \"Content-Type\": \"application/json\" }, body: JSON.stringify(body) });\n"
            "  const data = await res.json();\n"
            "  if (!res.ok) throw new Error(data.detail || `ACP facade failed (${res.status})`);\n"
            "  window.dispatchEvent(new CustomEvent(\"acp-delegation-started\", { detail: data }));\n"
            "  return `Delegation ${data.id} started with ${data.name}. Tell the user now that it has started and that you will report back. Do not claim it has finished yet.`;\n"
            "}\n\n"
            "/** @param {string} query @returns {Promise<string>} */\nasync function execWebSearch(query) {",
        ),
        (
            "    instructions: settings.instructions,",
            "    instructions: settings.instructions + \"\\n\\n\" + "
            + json.dumps(
                "DELEGATION POLICY: A verbal promise is not a delegation. If the user asks or tells "
                "a configured agent by name or alias to do something, you MUST call delegate_to_agent. "
                "Never say you asked, told, sent, started, or delegated unless the tool returned a real "
                "job ID. After the tool returns, report that the job started. " + facade.routing_guide()
            )
            + ",",
        ),
        (
            '  if (!s) return "";\n  if (!/^wss?:\\/\\//i.test(s)) {',
            '  if (!s) return "";\n'
            '  if (s.startsWith("/")) {\n'
            '    const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";\n'
            '    return `${scheme}//${window.location.host}${s}`;\n'
            '  }\n'
            '  if (!/^wss?:\\/\\//i.test(s)) {',
        ),
        (
            '  const message = err instanceof Error ? err.message : String(err);',
            '  const message = err instanceof Error\n'
            '    ? err.message\n'
            '    : err instanceof Event\n'
            '      ? "Realtime WebSocket connection failed."\n'
            '      : String(err);',
        ),
        (
            "/** @param {string} status */\nfunction onClientStatus(status) {",
            "const delegationAnnouncements = [];\n"
            "let delegationAnnouncementTimer = 0;\n"
            "function flushDelegationAnnouncements() {\n"
            "  if (!delegationAnnouncements.length) return;\n"
            "  if (!client || currentState !== \"listening\") {\n"
            "    if (!delegationAnnouncementTimer) delegationAnnouncementTimer = window.setTimeout(() => { delegationAnnouncementTimer = 0; flushDelegationAnnouncements(); }, 500);\n"
            "    return;\n"
            "  }\n"
            "  const job = delegationAnnouncements.shift();\n"
            "  const outcome = job.status === \"succeeded\"\n"
            "    ? `${job.name} completed the delegated task successfully. Result: ${job.result || \"Completed without a text result.\"}`\n"
            "    : `${job.name} failed the delegated task. Error: ${job.error || \"Unknown error.\"}`;\n"
            "  client.sendMessage(`Delegation status update: ${outcome} Tell the user this outcome now in a concise spoken response. Do not call delegate_to_agent again for this notification.`);\n"
            "  if (delegationAnnouncements.length && !delegationAnnouncementTimer) delegationAnnouncementTimer = window.setTimeout(() => { delegationAnnouncementTimer = 0; flushDelegationAnnouncements(); }, 500);\n"
            "}\n"
            "window.addEventListener(\"acp-delegation-complete\", (event) => {\n"
            "  delegationAnnouncements.push(event.detail);\n"
            "  flushDelegationAnnouncements();\n"
            "});\n\n"
            "/** @param {string} status */\nfunction onClientStatus(status) {",
        ),
    ]
    for marker, replacement in changes:
        if source.count(marker) != 1:
            raise RuntimeError(f"HF demo overlay marker changed: {marker[:60]!r}")
        source = source.replace(marker, replacement)
    return source


def _inject_realtime_client(source: str) -> str:
    marker = '  /** @param {{image?: string}} [options] */\n  requestResponse(options = {}) {'
    replacement = (
        "  /** Send a text event and request an audible model response. */\n"
        "  sendMessage(message) {\n"
        "    if (!this._session || !message?.trim()) return;\n"
        "    this._responseRequested = true;\n"
        "    this._session.sendMessage(message);\n"
        "  }\n\n"
        + marker
    )
    if source.count(marker) != 1:
        raise RuntimeError("HF realtime client overlay marker changed")
    return source.replace(marker, replacement)


@app.get("/main.js", include_in_schema=False)
async def main_js():
    await facade.refresh()
    return Response(
        _inject_acp_tool((DEMO / "main.js").read_text()),
        media_type="text/javascript",
        headers={"Cache-Control": "no-store"},
    )


@app.get("/s2s-realtime-client.js", include_in_schema=False)
async def realtime_client_js():
    return Response(
        _inject_realtime_client((DEMO / "s2s-realtime-client.js").read_text()),
        media_type="text/javascript",
        headers={"Cache-Control": "no-store"},
    )


@app.on_event("shutdown")
async def shutdown():
    await delegations.close()
    await facade.close()


app.mount("/", module.app)
