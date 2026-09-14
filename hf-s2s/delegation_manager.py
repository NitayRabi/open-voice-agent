"""In-memory lifecycle tracking for asynchronous ACP delegations."""
from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any
from uuid import uuid4

from acp_facade import ACPError, ACPFacade


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


class DelegationManager:
    def __init__(self, facade: ACPFacade, limit: int = 100):
        self.facade = facade
        self.limit = limit
        self.jobs: dict[str, dict[str, Any]] = {}
        self.tasks: set[asyncio.Task] = set()

    async def start(self, task: str, agent: str | None = None) -> dict[str, Any]:
        await self.facade.refresh()
        clean_task = task.strip()
        if not clean_task:
            raise ValueError("task must not be empty")
        requested = (agent or self.facade.default_agent).strip()
        provider_id = self.facade.resolve_provider(requested) if requested else ""
        if not provider_id or provider_id not in self.facade.agents:
            raise ValueError(f"Unknown ACP agent {requested!r}")
        provider = self.facade.agents[provider_id]
        job_id = uuid4().hex[:12]
        job = {
            "id": job_id,
            "status": "queued",
            "agent": provider_id,
            "name": provider.config["displayName"],
            "task": clean_task,
            "createdAt": _now(),
            "startedAt": None,
            "finishedAt": None,
            "result": None,
            "error": None,
        }
        self.jobs[job_id] = job
        while len(self.jobs) > self.limit:
            self.jobs.pop(next(iter(self.jobs)))
        background = asyncio.create_task(self._run(job_id, clean_task, provider_id))
        self.tasks.add(background)
        background.add_done_callback(self.tasks.discard)
        return dict(job)

    async def _run(self, job_id: str, task: str, provider_id: str) -> None:
        job = self.jobs[job_id]
        job["status"] = "running"
        job["startedAt"] = _now()
        try:
            response = await self.facade.delegate(task, provider_id)
            job["status"] = "succeeded"
            job["result"] = response["result"]
        except (ACPError, ValueError, OSError) as exc:
            job["status"] = "failed"
            job["error"] = str(exc)
        except Exception as exc:  # keep background failures visible to the user
            job["status"] = "failed"
            job["error"] = f"Unexpected delegation error: {exc}"
        finally:
            job["finishedAt"] = _now()

    def get(self, job_id: str) -> dict[str, Any]:
        if job_id not in self.jobs:
            raise ValueError(f"Unknown delegation {job_id!r}")
        return dict(self.jobs[job_id])

    def list(self) -> list[dict[str, Any]]:
        return [dict(job) for job in reversed(self.jobs.values())]

    async def close(self) -> None:
        for task in self.tasks:
            task.cancel()
        if self.tasks:
            await asyncio.gather(*self.tasks, return_exceptions=True)
