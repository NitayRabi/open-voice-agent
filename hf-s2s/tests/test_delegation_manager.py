import asyncio
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
MODULES = HERE.parent
sys.path.insert(0, str(MODULES))

from acp_facade import ACPFacade  # noqa: E402
from delegation_manager import DelegationManager  # noqa: E402


class DelegationManagerTests(unittest.IsolatedAsyncioTestCase):
    async def test_tracks_successful_delegation_lifecycle(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({
                "defaultAgent": "milo",
                "agents": {
                    "milo": {
                        "displayName": "Milo",
                        "aliases": ["OpenClaw"],
                        "command": sys.executable,
                        "args": [str(HERE / "fake_acp.py")],
                        "cwd": str(HERE),
                    }
                },
            }))
            facade = ACPFacade(config_path)
            manager = DelegationManager(facade)
            try:
                started = await manager.start("do the work", "openclaw")
                self.assertEqual(started["status"], "queued")
                self.assertEqual(started["name"], "Milo")
                for _ in range(50):
                    job = manager.get(started["id"])
                    if job["status"] == "succeeded":
                        break
                    await asyncio.sleep(0.01)
                self.assertEqual(job["status"], "succeeded")
                self.assertEqual(job["result"], "delegated successfully")
                self.assertIsNotNone(job["startedAt"])
                self.assertIsNotNone(job["finishedAt"])
            finally:
                await manager.close()
                await facade.close()

    async def test_records_launch_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({
                "defaultAgent": "broken",
                "agents": {"broken": {"command": "/does/not/exist"}},
            }))
            facade = ACPFacade(config_path)
            manager = DelegationManager(facade)
            try:
                started = await manager.start("fail visibly")
                for _ in range(50):
                    job = manager.get(started["id"])
                    if job["status"] == "failed":
                        break
                    await asyncio.sleep(0.01)
                self.assertEqual(job["status"], "failed")
                self.assertTrue(job["error"])
            finally:
                await manager.close()
                await facade.close()


if __name__ == "__main__":
    unittest.main()
