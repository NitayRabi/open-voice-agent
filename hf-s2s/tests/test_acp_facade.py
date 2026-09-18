import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
MODULES = HERE.parent
sys.path.insert(0, str(MODULES))

from acp_facade import ACPFacade  # noqa: E402


class ACPFacadeTests(unittest.IsolatedAsyncioTestCase):
    async def test_delegates_over_acp_and_collects_agent_chunks(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({
                "defaultAgent": "test",
                "agents": {
                    "test": {
                        "command": sys.executable,
                        "args": [str(HERE / "fake_acp.py")],
                        "cwd": str(HERE),
                        "timeoutSeconds": 2,
                    }
                },
            }))
            facade = ACPFacade(config_path)
            try:
                result = await facade.delegate("do something")
            finally:
                await facade.close()
            self.assertEqual(result, {
                "agent": "test",
                "name": "test",
                "result": "delegated successfully",
            })

    async def test_supports_zero_timeout_for_completion_driven_agent(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({
                "defaultAgent": "test",
                "agents": {
                    "test": {
                        "command": sys.executable,
                        "args": [str(HERE / "fake_acp.py")],
                        "cwd": str(HERE),
                        "timeoutSeconds": 0,
                    }
                },
            }))
            facade = ACPFacade(config_path)
            try:
                result = await facade.delegate("do something")
            finally:
                await facade.close()
            self.assertEqual(result, {
                "agent": "test",
                "name": "test",
                "result": "delegated successfully",
            })

    async def test_rejects_unknown_agent_without_starting_a_process(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({
                "defaultAgent": "test",
                "agents": {"test": {"command": "unused"}},
            }))
            facade = ACPFacade(config_path)
            with self.assertRaisesRegex(ValueError, "Unknown ACP agent"):
                await facade.delegate("work", "missing")

    async def test_provider_registry_can_add_select_and_remove_providers(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({"defaultAgent": "", "agents": {}}))
            facade = ACPFacade(config_path)
            try:
                registry = await facade.upsert_provider("custom", {
                    "displayName": "Milo",
                    "aliases": ["openclaw"],
                    "guidance": "Use for personal automation.",
                    "command": sys.executable,
                    "args": [str(HERE / "fake_acp.py")],
                    "cwd": str(HERE),
                    "timeoutSeconds": 5,
                    "env": {"SECRET_TOKEN": "not-returned"},
                })
                self.assertEqual(registry["defaultProvider"], "custom")
                self.assertEqual(registry["providers"][0]["envKeys"], ["SECRET_TOKEN"])
                self.assertNotIn("not-returned", json.dumps(registry))
                self.assertEqual(facade.resolve_provider("milo"), "custom")
                self.assertEqual(facade.resolve_provider("OPENCLAW"), "custom")
                self.assertEqual((await facade.delegate("test"))["result"], "delegated successfully")

                await facade.upsert_provider("second", {"command": "unused"})
                registry = await facade.set_default("second")
                self.assertEqual(registry["defaultProvider"], "second")
                registry = await facade.remove_provider("second")
                self.assertEqual(registry["defaultProvider"], "custom")
            finally:
                await facade.close()

    async def test_provider_aliases_must_be_unique(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({"defaultAgent": "", "agents": {}}))
            facade = ACPFacade(config_path)
            try:
                await facade.upsert_provider("first", {
                    "displayName": "Milo",
                    "command": "unused",
                })
                with self.assertRaisesRegex(ValueError, "already used"):
                    await facade.upsert_provider("second", {
                        "aliases": ["milo"],
                        "command": "unused",
                    })
            finally:
                await facade.close()

    async def test_second_facade_reloads_provider_file_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            config_path = Path(directory) / "agents.json"
            config_path.write_text(json.dumps({"defaultAgent": "", "agents": {}}))
            writer = ACPFacade(config_path)
            reader = ACPFacade(config_path)
            try:
                await writer.upsert_provider("shared", {"command": "unused"})
                await reader.refresh()
                self.assertEqual(reader.default_agent, "shared")
                self.assertIn("shared", reader.agents)
            finally:
                await writer.close()
                await reader.close()


if __name__ == "__main__":
    unittest.main()
