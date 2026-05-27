"""Engine — main entry point for running agents."""

import json
import subprocess
from typing import Any, Iterator, Optional

from datamirai.agent import Agent
from datamirai.types import ExecutionResult, ExecutionStatus, StreamEvent, TraceEntry


class Engine:
    """Mirai Engine client.

    Runs agents via the HTTP API (if server is running) or via the CLI binary.

    Args:
        provider: LLM provider name (ollama, openai, claude, etc.)
        model: Model identifier
        api_key: API key for the provider
        base_url: Custom API base URL
        server_url: URL of running mirai server (default: http://localhost:3000)
        binary_path: Path to mirai CLI binary (default: "mirai")
    """

    def __init__(
        self,
        provider: str = "ollama",
        model: Optional[str] = None,
        api_key: Optional[str] = None,
        base_url: Optional[str] = None,
        server_url: str = "http://localhost:3000",
        binary_path: str = "mirai",
    ):
        self.provider = provider
        self.model = model
        self.api_key = api_key
        self.base_url = base_url
        self.server_url = server_url.rstrip("/")
        self.binary_path = binary_path

    def run(
        self,
        agent: Agent,
        input: Optional[dict[str, Any]] = None,
        timeout: int = 300,
    ) -> ExecutionResult:
        """Execute an agent and return the result.

        Tries HTTP API first, falls back to CLI binary.
        """
        try:
            return self._run_via_http(agent, input, timeout)
        except Exception:
            return self._run_via_cli(agent, input, timeout)

    def stream(
        self,
        agent: Agent,
        input: Optional[dict[str, Any]] = None,
    ) -> Iterator[StreamEvent]:
        """Execute an agent with streaming, yielding events as they arrive.

        Requires the HTTP server to be running.
        """
        try:
            import requests
        except ImportError:
            raise ImportError("requests is required for streaming: pip install requests")

        # First, load the agent via from-spec endpoint.
        resp = requests.post(
            f"{self.server_url}/api/agents/from-spec",
            json=agent.spec,
            timeout=10,
        )
        resp.raise_for_status()
        agent_id = resp.json()["id"]

        # Then stream execution.
        resp = requests.post(
            f"{self.server_url}/api/agents/{agent_id}/stream",
            json={"trigger_data": input or {}},
            stream=True,
            headers={"Accept": "text/event-stream"},
            timeout=300,
        )

        current_event = ""
        current_data = ""
        for line in resp.iter_lines(decode_unicode=True):
            if line is None:
                continue
            line = line.strip()
            if line.startswith("event: "):
                current_event = line[7:]
            elif line.startswith("data: "):
                current_data = line[6:]
            elif line == "" and current_event:
                try:
                    data = json.loads(current_data) if current_data else {}
                except json.JSONDecodeError:
                    data = {"raw": current_data}
                yield StreamEvent(event=current_event, data=data)
                current_event = ""
                current_data = ""

    def _run_via_http(
        self,
        agent: Agent,
        input: Optional[dict[str, Any]],
        timeout: int,
    ) -> ExecutionResult:
        """Run via HTTP API."""
        try:
            import requests
        except ImportError:
            raise RuntimeError("requests not available")

        # Load agent.
        resp = requests.post(
            f"{self.server_url}/api/agents/from-spec",
            json=agent.spec,
            timeout=10,
        )
        resp.raise_for_status()
        agent_id = resp.json()["id"]

        # Execute.
        resp = requests.post(
            f"{self.server_url}/api/agents/{agent_id}/execute",
            json={"trigger_data": input or {}},
            timeout=timeout,
        )
        resp.raise_for_status()
        data = resp.json()

        return self._parse_result(data)

    def _run_via_cli(
        self,
        agent: Agent,
        input: Optional[dict[str, Any]],
        timeout: int,
    ) -> ExecutionResult:
        """Run via CLI binary."""
        import tempfile
        import os

        # Write spec to temp file.
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False
        ) as f:
            json.dump(agent.spec, f)
            temp_path = f.name

        try:
            cmd = [self.binary_path, "run", temp_path]
            cmd.extend(["--provider", self.provider])
            if self.model:
                cmd.extend(["--model", self.model])
            if self.api_key:
                cmd.extend(["--api-key", self.api_key])
            if self.base_url:
                cmd.extend(["--base-url", self.base_url])
            if input:
                cmd.extend(["--input", json.dumps(input)])

            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=timeout,
            )

            if result.returncode == 0:
                data = json.loads(result.stdout)
                return self._parse_result(data)
            else:
                try:
                    data = json.loads(result.stdout)
                    return self._parse_result(data)
                except json.JSONDecodeError:
                    return ExecutionResult(
                        status=ExecutionStatus.FAILED,
                        error=result.stderr or result.stdout,
                    )
        finally:
            os.unlink(temp_path)

    def _parse_result(self, data: dict) -> ExecutionResult:
        """Parse raw JSON result into ExecutionResult."""
        trace = [
            TraceEntry(
                node_id=t.get("node_id", ""),
                tool_type=t.get("tool_type", ""),
                status=t.get("status", ""),
                duration_ms=t.get("duration_ms", 0),
                retries=t.get("retries", 0),
                error=t.get("error"),
            )
            for t in data.get("trace", [])
        ]

        status_str = str(data.get("status", "Failed"))
        try:
            status = ExecutionStatus(status_str)
        except ValueError:
            status = ExecutionStatus.FAILED

        return ExecutionResult(
            status=status,
            state=data.get("state", {}),
            trace=trace,
            transcript=data.get("transcript", []),
            error=data.get("error"),
        )
