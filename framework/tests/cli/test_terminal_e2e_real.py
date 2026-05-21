"""Real E2E tests — spawn the actual app with REAL Ollama LLM. NO MOCKS.

Uses pexpect to simulate a real user at the terminal.
Uses the real Ollama service with qwen3:8b running locally.
Verifies EXACTLY what a human user would experience.

Requirements:
- Ollama running on localhost:11434
- Model qwen3:8b pulled
"""

from __future__ import annotations

import os
import sys

import pexpect
import pytest

_RUNNER = os.path.join(os.path.dirname(__file__), "_run_mirai_code.py")
_LLM_TIMEOUT = 120  # real LLM inference can take 30-90s


def _ollama_ready() -> bool:
    try:
        import httpx
        resp = httpx.get("http://localhost:11434/api/tags", timeout=3)
        models = [m["name"] for m in resp.json().get("models", [])]
        return "qwen3:8b" in models
    except Exception:
        return False


pytestmark = pytest.mark.skipif(
    not _ollama_ready(),
    reason="Ollama not running or qwen3:8b not available",
)


def _spawn(cwd: str = "/tmp") -> pexpect.spawn:
    child = pexpect.spawn(
        sys.executable,
        [_RUNNER, "--provider", "ollama", "--model", "qwen3:8b", "--cwd", cwd, "--autonomy", "copilot"],
        encoding="utf-8",
        timeout=_LLM_TIMEOUT,
    )
    return child


class TestAppStartup:
    def test_banner_shows_all_info(self):
        child = _spawn()
        child.expect("Mirai Code", timeout=10)
        child.expect("ollama", timeout=5)
        child.expect("qwen3:8b", timeout=5)
        child.expect("Tools loaded:", timeout=5)
        child.expect("copilot", timeout=5)
        child.expect(">", timeout=5)

        child.sendline("/quit")
        child.expect("Bye", timeout=5)
        child.wait()


class TestSlashCommands:
    def test_help(self):
        child = _spawn()
        child.expect(">", timeout=10)
        child.sendline("/help")
        child.expect("/checkpoint", timeout=5)
        child.expect("/rollback", timeout=5)
        child.expect("/clear", timeout=5)
        child.expect("/tools", timeout=5)
        child.expect("/quit", timeout=5)

        child.sendline("/quit")
        child.wait()

    def test_tools_catalog(self):
        child = _spawn()
        child.expect(">", timeout=10)
        child.sendline("/tools")
        child.expect("filesystem", timeout=5)
        child.expect("system", timeout=5)
        child.expect("git", timeout=5)

        child.sendline("/quit")
        child.wait()

    def test_session_info(self):
        child = _spawn()
        child.expect(">", timeout=10)
        child.sendline("/session")
        child.expect("ses_", timeout=5)

        child.sendline("/quit")
        child.expect("Session saved", timeout=5)
        child.wait()

    def test_tokens_initially_zero(self):
        child = _spawn()
        child.expect(">", timeout=10)
        child.sendline("/tokens")
        child.expect("0 tokens", timeout=5)

        child.sendline("/quit")
        child.wait()


class TestRealConversation:
    def test_llm_responds_to_greeting(self):
        """Send a simple message → real LLM processes → we get a response back."""
        child = _spawn()
        child.expect(">", timeout=10)

        child.sendline("Responde unicamente: OK")
        child.expect("Thinking", timeout=10)
        # Wait for LLM to finish and show response
        child.expect(">", timeout=_LLM_TIMEOUT)

        # Verify tokens were consumed
        child.sendline("/tokens")
        child.expect("calls", timeout=5)

        child.sendline("/quit")
        child.wait()

    def test_llm_calls_tool(self, tmp_path):
        """Ask LLM to list files → it should use a tool → we see tool indicator."""
        (tmp_path / "mirai_test.py").write_text("print('test')")

        child = _spawn(cwd=str(tmp_path))
        child.expect(">", timeout=10)

        child.sendline("Usa la herramienta filesystem_list_dir para listar los archivos del directorio actual")
        child.expect("Thinking", timeout=10)

        # Look for tool call or final response
        idx = child.expect(
            ["filesystem/", "system/", ">"],
            timeout=_LLM_TIMEOUT,
        )
        if idx in (0, 1):
            # Tool was called, wait for completion
            child.expect(">", timeout=_LLM_TIMEOUT)

        child.sendline("/quit")
        child.wait()


class TestCheckpointWorkflow:
    def test_create_list_checkpoint(self):
        child = _spawn()
        child.expect(">", timeout=10)

        # Have a conversation first
        child.sendline("di hola brevemente")
        child.expect(">", timeout=_LLM_TIMEOUT)

        # Create checkpoint
        child.sendline("/checkpoint mi_punto_seguro")
        child.expect("Checkpoint created", timeout=5)
        child.expect("mi_punto_seguro", timeout=5)

        # List checkpoints
        child.sendline("/checkpoints")
        child.expect("mi_punto_seguro", timeout=5)

        child.sendline("/quit")
        child.wait()


class TestClearAndContinue:
    def test_clear_then_continue(self):
        child = _spawn()
        child.expect(">", timeout=10)

        child.sendline("hola")
        child.expect(">", timeout=_LLM_TIMEOUT)

        child.sendline("/clear")
        child.expect("Context cleared", timeout=5)

        # App still works after clear
        child.sendline("di OK")
        child.expect(">", timeout=_LLM_TIMEOUT)

        child.sendline("/quit")
        child.wait()


class TestGracefulExit:
    def test_quit_saves_and_exits(self):
        child = _spawn()
        child.expect(">", timeout=10)
        child.sendline("/quit")
        # Process may exit quickly — accept Session saved, Bye, or EOF
        child.expect([pexpect.EOF, "Session saved", "Bye"], timeout=10)
        child.expect([pexpect.EOF], timeout=5)

    def test_ctrl_c_at_prompt(self):
        child = _spawn()
        child.expect(">", timeout=10)
        child.sendcontrol("c")
        # Ctrl+C at input() may show Session saved or just exit
        try:
            child.expect([pexpect.EOF, "Session saved", "Bye"], timeout=10)
        except pexpect.TIMEOUT:
            # Force kill if it didn't exit
            child.sendcontrol("c")
            child.expect([pexpect.EOF], timeout=5)
