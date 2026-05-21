"""Mock Ollama server for E2E terminal tests.

Starts a tiny HTTP server that mimics Ollama's /api/chat endpoint
with predictable responses. Supports tool calling simulation.
"""

from __future__ import annotations

import json
import threading
from http.server import HTTPServer, BaseHTTPRequestHandler
from typing import Any


class _Handler(BaseHTTPRequestHandler):
    """Handles /api/chat and /api/tags."""

    # Class-level response queue (set by tests)
    responses: list[dict[str, Any]] = []
    response_index: int = 0

    def do_GET(self):
        if self.path == "/api/tags":
            self._json_response({"models": [
                {"name": "mock-model", "details": {"parameter_size": "1B"}},
            ]})
        elif self.path == "/":
            self._text_response("Ollama is running")
        else:
            self.send_error(404)

    def do_POST(self):
        if self.path == "/api/chat":
            content_length = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(content_length)) if content_length else {}

            # Get next response from queue
            if _Handler.response_index < len(_Handler.responses):
                resp_data = _Handler.responses[_Handler.response_index]
                _Handler.response_index += 1
            else:
                # Default: simple text response
                resp_data = {
                    "message": {"role": "assistant", "content": "Mock response."},
                    "model": "mock-model",
                    "prompt_eval_count": 10,
                    "eval_count": 5,
                }

            self._json_response(resp_data)

        elif self.path == "/api/show":
            self._json_response({
                "model_info": {"general.context_length": 4096},
                "details": {"parameter_size": "1B", "family": "mock"},
            })
        else:
            self.send_error(404)

    def _json_response(self, data: dict):
        body = json.dumps(data).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _text_response(self, text: str):
        body = text.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        pass  # Suppress logs during tests


def start_mock_server(port: int = 0, responses: list[dict] | None = None) -> tuple[HTTPServer, int]:
    """Start a mock Ollama server on a random port. Returns (server, port)."""
    _Handler.responses = responses or []
    _Handler.response_index = 0

    server = HTTPServer(("127.0.0.1", port), _Handler)
    actual_port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, actual_port


def make_text_response(text: str) -> dict:
    """Create a mock Ollama response with just text."""
    return {
        "message": {"role": "assistant", "content": text},
        "model": "mock-model",
        "prompt_eval_count": 10,
        "eval_count": 5,
    }


def make_tool_call_response(tool_name: str, args: dict) -> dict:
    """Create a mock Ollama response with a tool call."""
    return {
        "message": {
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {"function": {"name": tool_name, "arguments": args}},
            ],
        },
        "model": "mock-model",
        "prompt_eval_count": 15,
        "eval_count": 8,
    }
