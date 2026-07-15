#!/usr/bin/env python3
"""E2E Test Suite for OpenMirai Python SDK."""

import os
import subprocess
import time
import unittest
import requests

from openmirai.agent import Agent
from openmirai.engine import Engine


class TestPythonSDK(unittest.TestCase):
    server_process = None
    port = 8086
    server_url = f"http://127.0.0.1:{port}"
    binary_path = "../../target/debug/mirai"

    @classmethod
    def setUpClass(cls):
        # Verify CLI binary exists
        if not os.path.exists(cls.binary_path):
            raise RuntimeError(f"mirai binary not found at {cls.binary_path}. Run cargo build first.")

        # Start HTTP server in background
        print(f"Starting server on {cls.server_url} for E2E tests...")
        cls.server_process = subprocess.Popen(
            [cls.binary_path, "serve", "--port", str(cls.port), "--provider", "mock"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL
        )

        # Wait for server to become healthy
        healthy = False
        for _ in range(20):
            try:
                resp = requests.get(f"{cls.server_url}/health", timeout=1)
                if resp.status_code == 200 and resp.json().get("status") == "ok":
                    healthy = True
                    break
            except requests.RequestException:
                pass
            time.sleep(0.5)

        if not healthy:
            cls.tearDownClass()
            raise RuntimeError("Server failed to start or report healthy in time.")

    @classmethod
    def tearDownClass(cls):
        if cls.server_process:
            print("Terminating server process...")
            cls.server_process.terminate()
            cls.server_process.wait()

    def setUp(self):
        self.engine = Engine(
            provider="mock",
            server_url=self.server_url,
            binary_path=self.binary_path
        )
        self.agent = Agent(spec={
            "name": "py-test-agent",
            "description": "Python SDK E2E test",
            "version": "v1",
            "graph": {
                "nodes": [
                    {
                        "id": "start",
                        "tool_type": "trigger/manual",
                        "config": {
                            "payload": {
                                "value": "hello"
                            }
                        }
                    },
                    {
                        "id": "respond",
                        "tool_type": "output/response",
                        "config": {
                            "message": "python ok"
                        }
                    }
                ],
                "edges": [
                    {
                        "source": "start",
                        "target": "respond"
                    }
                ]
            }
        })

    def test_run_via_http(self):
        print("Testing HTTP run...")
        res = self.engine._run_via_http(self.agent, input={}, timeout=30)
        self.assertEqual(res.status.value, "Completed")
        self.assertIn("respond", res.state)
        self.assertEqual(res.state["respond"]["raw_data"]["message"], "python ok")

    def test_run_via_cli(self):
        print("Testing CLI run...")
        res = self.engine._run_via_cli(self.agent, input={}, timeout=30)
        if res.status.value != "Completed":
            print(f"CLI run failed with error: {res.error}")
            print(f"CLI run state: {res.state}")
            print(f"CLI run transcript: {res.transcript}")
        self.assertEqual(res.status.value, "Completed")
        self.assertIn("respond", res.state)
        self.assertEqual(res.state["respond"]["raw_data"]["message"], "python ok")

    def test_run_autodetect(self):
        print("Testing autodetect fallback run...")
        res = self.engine.run(self.agent, input={})
        self.assertEqual(res.status.value, "Completed")
        self.assertEqual(res.state["respond"]["raw_data"]["message"], "python ok")

    def test_streaming(self):
        print("Testing streaming execution...")
        events = list(self.engine.stream(self.agent, input={}))
        self.assertTrue(len(events) > 0)
        
        # Check that we received completed or node-related events
        event_names = [e.event for e in events]
        self.assertIn("graph.completed", event_names)


if __name__ == "__main__":
    unittest.main()
