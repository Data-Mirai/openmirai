"""interactive_agent_terminal — the agentic loop that powers Mirai Code.

This is the core interactive terminal where a user talks to an LLM that
has access to the full Mirai Engine tool catalog. The LLM reasons,
calls tools, gets results, and repeats until the task is done.

Usage:
    datamirai-app code [--provider groq] [--model qwen-qwq-32b] [--cwd .]
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import readline  # enables arrow keys, history, cursor movement in input()
import sys
import time
from typing import Any

from datamirai_engine.llm.adapter import LLMAdapter, NormalizedResponse
from datamirai_engine.tools.registry import ToolRegistry

from datamirai_engine.cli.autonomy_levels import (
    AutonomyConfig,
    ask_user_confirmation,
    get_autonomy_config,
)
from datamirai_engine.cli.system_prompt_builder import build_system_prompt
from datamirai_engine.cli.tool_schema_builder import (
    build_tool_schemas,
    execute_tool,
    get_tool_name_map,
)


# ---------------------------------------------------------------------------
# ANSI colors
# ---------------------------------------------------------------------------

class _C:
    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"
    CYAN = "\033[36m"
    GREEN = "\033[32m"
    YELLOW = "\033[33m"
    RED = "\033[31m"
    MAGENTA = "\033[35m"
    BLUE = "\033[34m"
    BRIGHT_RED = "\033[91m"
    BG_RED = "\033[41m"
    WHITE = "\033[37m"


# ---------------------------------------------------------------------------
# Sticky status bar at the bottom of the terminal
# ---------------------------------------------------------------------------

class StatusBar:
    """Renders a persistent status bar at the bottom of the terminal."""

    def __init__(self, max_ctx: int = 131072) -> None:
        self.max_ctx = max_ctx
        self._enabled = True

    def _get_width(self) -> int:
        try:
            return os.get_terminal_size().columns
        except OSError:
            return 80

    def _get_height(self) -> int:
        try:
            return os.get_terminal_size().lines
        except OSError:
            return 24

    def _ctx_color(self, pct: int) -> str:
        """Color gradient: green → yellow → orange → red → dark red."""
        if pct < 20:
            return _C.GREEN
        if pct < 40:
            return "\033[32m"  # green
        if pct < 55:
            return _C.YELLOW
        if pct < 65:
            return _C.BRIGHT_RED
        if pct < 75:
            return _C.RED
        return f"{_C.BG_RED}{_C.WHITE}"  # critical: white on red background

    def _bar_chars(self, pct: int, width: int = 15) -> str:
        filled = int(width * min(pct, 100) / 100)
        empty = width - filled
        color = self._ctx_color(pct)
        return f"{color}{'█' * filled}{_C.DIM}{'░' * empty}{_C.RESET}"

    def render(
        self,
        ctx_used: int,
        total_tokens: int,
        msg_count: int,
        model: str = "",
        autonomy: str = "",
    ) -> None:
        """Draw the status bar at the bottom of the terminal."""
        if not self._enabled:
            return

        pct = min(100, int(ctx_used / self.max_ctx * 100)) if self.max_ctx > 0 else 0
        color = self._ctx_color(pct)
        bar = self._bar_chars(pct)
        width = self._get_width()
        height = self._get_height()

        # Build status text
        ctx_text = f"{color}{pct}%{_C.RESET} ({ctx_used:,}/{self.max_ctx:,})"
        tokens_text = f"tok:{total_tokens:,}"
        msgs_text = f"msgs:{msg_count}"

        status_parts = [f" {bar} {ctx_text}", tokens_text, msgs_text]
        if model:
            status_parts.append(model)
        if autonomy:
            status_parts.append(autonomy)

        status = f"{_C.DIM} | {_C.RESET}".join(status_parts)

        # Save cursor, move to bottom, print, restore cursor
        sys.stdout.write(f"\033[s")  # save cursor
        sys.stdout.write(f"\033[{height};1H")  # move to last line
        sys.stdout.write(f"\033[2K")  # clear line
        sys.stdout.write(f"\033[7m {_C.RESET}")  # subtle background
        sys.stdout.write(status)
        # Pad to full width
        visible_len = len(status) - sum(len(c) for c in [_C.RESET, _C.DIM, color, _C.RESET] * 3)
        padding = max(0, width - visible_len - 2)
        sys.stdout.write(" " * padding)
        sys.stdout.write(f"{_C.RESET}")
        sys.stdout.write(f"\033[u")  # restore cursor
        sys.stdout.flush()

    def setup_scroll_region(self) -> None:
        """Reserve the bottom line by setting scroll region."""
        height = self._get_height()
        sys.stdout.write(f"\033[1;{height - 1}r")  # scroll region = line 1 to height-1
        sys.stdout.write(f"\033[{height - 1};1H")  # move cursor to bottom of scroll region
        sys.stdout.flush()

    def cleanup(self) -> None:
        """Restore full scroll region on exit."""
        height = self._get_height()
        sys.stdout.write(f"\033[1;{height}r")  # restore full scroll
        sys.stdout.write(f"\033[{height};1H")  # move to bottom
        sys.stdout.write(f"\033[2K")  # clear status line
        sys.stdout.write(f"\033[{height - 1};1H")  # move up
        sys.stdout.flush()


# ---------------------------------------------------------------------------
# Adapter factory
# ---------------------------------------------------------------------------

def _create_adapter(provider: str, api_key: str = "", base_url: str = "") -> LLMAdapter:
    """Instantiate the right adapter for the chosen provider."""
    if provider == "groq":
        from datamirai_engine.llm.adapters.groq import GroqAdapter
        key = api_key or os.environ.get("GROQ_API_KEY", "")
        if not key:
            print(f"{_C.RED}Error: Set GROQ_API_KEY env var or pass --api-key{_C.RESET}")
            sys.exit(1)
        return GroqAdapter(api_key=key, timeout=120.0)

    if provider == "nvidia":
        from datamirai_engine.llm.adapters.nvidia import NvidiaAdapter
        key = api_key or os.environ.get("NVIDIA_API_KEY", "")
        if not key:
            print(f"{_C.RED}Error: Set NVIDIA_API_KEY env var or pass --api-key{_C.RESET}")
            sys.exit(1)
        return NvidiaAdapter(api_key=key, timeout=120.0)

    if provider == "ollama":
        from datamirai_engine.llm.adapters.ollama import OllamaAdapter
        url = base_url or os.environ.get("OLLAMA_BASE_URL", "http://localhost:11434")
        return OllamaAdapter(base_url=url, timeout=300.0)

    if provider == "openai":
        from datamirai_engine.llm.adapters.openai_adapter import OpenAIAdapter
        key = api_key or os.environ.get("OPENAI_API_KEY", "")
        if not key:
            print(f"{_C.RED}Error: Set OPENAI_API_KEY env var or pass --api-key{_C.RESET}")
            sys.exit(1)
        return OpenAIAdapter(api_key=key, timeout=120.0)

    if provider == "openrouter":
        from datamirai_engine.llm.adapters.openrouter import OpenRouterAdapter
        key = api_key or os.environ.get("OPENROUTER_API_KEY", "")
        if not key:
            print(f"{_C.RED}Error: Set OPENROUTER_API_KEY env var or pass --api-key{_C.RESET}")
            sys.exit(1)
        return OpenRouterAdapter(api_key=key, timeout=120.0)

    print(f"{_C.RED}Unknown provider: {provider}. Use: groq, nvidia, ollama, openai, openrouter{_C.RESET}")
    sys.exit(1)


# ---------------------------------------------------------------------------
# Registry builder — full engine tool catalog
# ---------------------------------------------------------------------------

def _create_full_registry() -> ToolRegistry:
    """Register the complete catalog of engine tools."""
    registry = ToolRegistry()
    modules = [
        # --- filesystem ---
        "datamirai_engine.tools.builtin.filesystem.read_file",
        "datamirai_engine.tools.builtin.filesystem.write_file",
        "datamirai_engine.tools.builtin.filesystem.edit_file",
        "datamirai_engine.tools.builtin.filesystem.glob_files",
        "datamirai_engine.tools.builtin.filesystem.grep_files",
        "datamirai_engine.tools.builtin.filesystem.list_dir",
        "datamirai_engine.tools.builtin.filesystem.tree",
        "datamirai_engine.tools.builtin.filesystem.file_info",
        "datamirai_engine.tools.builtin.filesystem.move",
        "datamirai_engine.tools.builtin.filesystem.copy",
        "datamirai_engine.tools.builtin.filesystem.delete",
        "datamirai_engine.tools.builtin.filesystem.mkdir",
        # --- system ---
        "datamirai_engine.tools.builtin.system.bash",
        "datamirai_engine.tools.builtin.system.process_list",
        # --- git ---
        "datamirai_engine.tools.builtin.git.status",
        "datamirai_engine.tools.builtin.git.diff",
        "datamirai_engine.tools.builtin.git.log",
        "datamirai_engine.tools.builtin.git.commit",
        # --- logic ---
        "datamirai_engine.tools.builtin.logic.condition",
        "datamirai_engine.tools.builtin.logic.switch",
        "datamirai_engine.tools.builtin.logic.loop",
        "datamirai_engine.tools.builtin.logic.merge",
        "datamirai_engine.tools.builtin.logic.wait",
        "datamirai_engine.tools.builtin.logic.deadline",
        "datamirai_engine.tools.builtin.logic.human_input",
        # --- ai ---
        "datamirai_engine.tools.builtin.ai.llm_call",
        "datamirai_engine.tools.builtin.ai.transcribe",
        "datamirai_engine.tools.builtin.ai.embeddings",
        # --- data ---
        "datamirai_engine.tools.builtin.data.db_read",
        "datamirai_engine.tools.builtin.data.db_write",
        "datamirai_engine.tools.builtin.data.storage_read",
        "datamirai_engine.tools.builtin.data.storage_write",
        "datamirai_engine.tools.builtin.data.web_scrape",
        "datamirai_engine.tools.builtin.data.vault_write",
        "datamirai_engine.tools.builtin.data.vault_read",
        "datamirai_engine.tools.builtin.data.entity_upsert",
        "datamirai_engine.tools.builtin.data.entity_query",
        "datamirai_engine.tools.builtin.data.html_to_markdown",
        "datamirai_engine.tools.builtin.data.stealth",
        "datamirai_engine.tools.builtin.data.browser_agent",
        # --- agent ---
        "datamirai_engine.tools.builtin.agent.run_agent",
        # --- mcp ---
        "datamirai_engine.tools.builtin.mcp.mcp_call",
        # --- triggers & output (registered but excluded from schemas) ---
        "datamirai_engine.tools.builtin.trigger.triggers",
        "datamirai_engine.tools.builtin.trigger.heartbeat",
        "datamirai_engine.tools.builtin.output.response",
    ]
    for mod in modules:
        with contextlib.suppress(Exception):
            registry.discover(mod)
    return registry


# ---------------------------------------------------------------------------
# Token tracking
# ---------------------------------------------------------------------------

class TokenTracker:
    def __init__(self) -> None:
        self.total_input = 0
        self.total_output = 0
        self.calls = 0

    def add(self, tokens: dict[str, int]) -> None:
        self.total_input += tokens.get("input", 0)
        self.total_output += tokens.get("output", 0)
        self.calls += 1

    def summary(self) -> str:
        total = self.total_input + self.total_output
        return f"{total:,} tokens ({self.total_input:,} in / {self.total_output:,} out) across {self.calls} calls"


# ---------------------------------------------------------------------------
# Spinner — visual feedback while LLM is thinking
# ---------------------------------------------------------------------------

class _Spinner:
    _FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

    def __init__(self, label: str = "Thinking") -> None:
        self.label = label
        self._task: asyncio.Task[None] | None = None

    async def _spin(self) -> None:
        i = 0
        start = time.monotonic()
        try:
            while True:
                elapsed = time.monotonic() - start
                frame = self._FRAMES[i % len(self._FRAMES)]
                sys.stdout.write(f"\r  {_C.MAGENTA}{frame}{_C.RESET} {_C.DIM}{self.label}... ({elapsed:.0f}s){_C.RESET}  ")
                sys.stdout.flush()
                i += 1
                await asyncio.sleep(0.1)
        except asyncio.CancelledError:
            sys.stdout.write("\r" + " " * 60 + "\r")
            sys.stdout.flush()

    def start(self) -> None:
        self._task = asyncio.create_task(self._spin())

    def stop(self) -> None:
        if self._task:
            self._task.cancel()
            self._task = None


# ---------------------------------------------------------------------------
# The agentic loop — LLM → tool calls → execute → feed back → repeat
# ---------------------------------------------------------------------------

_MAX_TOOL_ROUNDS = 25


async def _agentic_loop(
    adapter: LLMAdapter,
    model: str,
    messages: list[dict[str, Any]],
    tools: list[dict[str, Any]],
    name_map: dict[str, str],
    registry: ToolRegistry,
    cwd: str,
    tracker: TokenTracker,
    temperature: float,
    max_tokens: int,
    storage: Any = None,
    session_id: str = "",
    autonomy: AutonomyConfig | None = None,
    context_window: int | None = None,
) -> tuple[str, bool]:
    """Run the agentic loop until the LLM responds with text (no tool calls).

    Returns (response_text, was_streamed).
    Ctrl+C during the LLM call cancels it and returns to the prompt.
    """
    if autonomy is None:
        autonomy = get_autonomy_config("copilot")

    # Extra kwargs for provider-specific params (e.g. num_ctx for Ollama)
    llm_kwargs: dict[str, Any] = {}
    if context_window:
        llm_kwargs["num_ctx"] = context_window

    max_rounds = autonomy.max_tool_rounds
    was_streamed = False

    for round_num in range(max_rounds):
        # Use streaming if adapter supports it — show tokens in real-time
        try:
            has_streaming = (
                hasattr(adapter, "stream_with_messages")
                and hasattr(adapter.__class__, "stream_with_messages")
                and adapter.__class__.stream_with_messages is not LLMAdapter.stream_with_messages
            )
        except (AttributeError, TypeError):
            has_streaming = False

        # Streaming now works with tools (tool_calls parsed from any chunk)

        if has_streaming:
            if round_num > 0:
                print(f"  {_C.DIM}(round {round_num + 1}){_C.RESET}")

            # Streaming state tracker
            _stream_state = {
                "in_think": False,
                "think_chars": 0,
                "response_started": False,
                "token_count": 0,
                "first_token_received": False,
                "start_time": time.monotonic(),
            }

            _SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

            # Show waiting indicator immediately
            sys.stdout.write(f"\r  {_C.MAGENTA}{_SPINNER_FRAMES[0]}{_C.RESET} {_C.DIM}Generating...{_C.RESET}")
            sys.stdout.flush()

            # Background task to animate spinner until first token
            async def _animate_waiting() -> None:
                i = 0
                while not _stream_state["first_token_received"]:
                    elapsed = time.monotonic() - _stream_state["start_time"]
                    frame = _SPINNER_FRAMES[i % len(_SPINNER_FRAMES)]
                    sys.stdout.write(
                        f"\r  {_C.MAGENTA}{frame}{_C.RESET} "
                        f"{_C.DIM}Generating... ({elapsed:.0f}s){_C.RESET}  "
                    )
                    sys.stdout.flush()
                    i += 1
                    await asyncio.sleep(0.1)

            waiting_task = asyncio.create_task(_animate_waiting())

            def _on_token(token: str) -> None:
                s = _stream_state
                s["token_count"] += 1
                buf = token

                # Stop the waiting spinner on first token
                if not s["first_token_received"]:
                    s["first_token_received"] = True
                    sys.stdout.write("\r" + " " * 80 + "\r")
                    sys.stdout.flush()

                # Detect <think> open
                if "<think>" in buf:
                    s["in_think"] = True
                    s["think_chars"] = 0
                    buf = buf.replace("<think>", "")

                # Detect </think> close
                if "</think>" in buf:
                    s["in_think"] = False
                    buf = buf.replace("</think>", "")
                    sys.stdout.write("\r" + " " * 80 + "\r")
                    sys.stdout.flush()

                if s["in_think"]:
                    s["think_chars"] += len(buf)
                    elapsed = time.monotonic() - s["start_time"]
                    frame = _SPINNER_FRAMES[int(elapsed * 10) % len(_SPINNER_FRAMES)]
                    sys.stdout.write(
                        f"\r  {_C.MAGENTA}{frame}{_C.RESET} "
                        f"{_C.DIM}Reasoning... ({s['think_chars']} chars, {elapsed:.0f}s){_C.RESET}  "
                    )
                    sys.stdout.flush()
                else:
                    # Actual response content — print it
                    if buf.strip() or s["response_started"]:
                        if not s["response_started"]:
                            s["response_started"] = True
                            sys.stdout.write(f"\n")
                        sys.stdout.write(buf)
                        sys.stdout.flush()

            try:
                response: NormalizedResponse = await adapter.stream_with_messages(
                    model=model,
                    messages=messages,
                    tools=tools,
                    temperature=temperature,
                    max_tokens=max_tokens,
                    on_token=_on_token,
                    **llm_kwargs,
                )
            except (KeyboardInterrupt, asyncio.CancelledError):
                _stream_state["first_token_received"] = True
                waiting_task.cancel()
                sys.stdout.write(f"{_C.RESET}\n")
                raise KeyboardInterrupt("LLM call interrupted by user")
            finally:
                _stream_state["first_token_received"] = True
                waiting_task.cancel()

            sys.stdout.write(f"{_C.RESET}")
            if _stream_state["response_started"] and not response.tool_calls:
                sys.stdout.write("\n")
            elif not _stream_state["response_started"] and not response.tool_calls:
                # Model only produced think tags, clear the line
                sys.stdout.write("\r" + " " * 80 + "\r")
            sys.stdout.flush()
        else:
            # Fallback: spinner for adapters without streaming
            label = "Thinking" if round_num == 0 else f"Thinking (round {round_num + 1})"
            spinner = _Spinner(label)
            spinner.start()
            try:
                response: NormalizedResponse = await adapter.call_with_messages(
                    model=model,
                    messages=messages,
                    tools=tools,
                    temperature=temperature,
                    max_tokens=max_tokens,
                    **llm_kwargs,
                )
            except (KeyboardInterrupt, asyncio.CancelledError):
                spinner.stop()
                raise KeyboardInterrupt("LLM call interrupted by user")
            finally:
                spinner.stop()

        tracker.add(response.tokens_used)
        was_streamed = has_streaming and _stream_state.get("response_started", False) if has_streaming else False

        if not response.tool_calls:
            return response.response, was_streamed

        assistant_msg: dict[str, Any] = {"role": "assistant", "content": response.response or None}
        assistant_msg["tool_calls"] = [
            {
                "id": tc.id,
                "type": "function",
                "function": {"name": tc.name, "arguments": tc.arguments},
            }
            for tc in response.tool_calls
        ]
        messages.append(assistant_msg)

        for tc in response.tool_calls:
            tool_type = name_map.get(tc.name, tc.name)
            try:
                args = json.loads(tc.arguments) if tc.arguments else {}
            except json.JSONDecodeError:
                args = {}

            print(f"  {_C.CYAN}▶ {tool_type}{_C.RESET} {_C.DIM}{_format_args(args)}{_C.RESET}")

            # Autonomy: confirm writes if required
            if autonomy.needs_confirmation(tool_type):
                if not ask_user_confirmation(tool_type, args):
                    print(f"    {_C.YELLOW}⏭ Skipped by user{_C.RESET}")
                    messages.append({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": json.dumps({"error": "User declined this action"}),
                    })
                    continue

            if storage and session_id:
                storage.append_tool_call(session_id, tool_type, args, round_num=round_num)

            start = time.monotonic()
            result = await execute_tool(registry, tool_type, args, cwd=cwd)
            elapsed = time.monotonic() - start

            if storage and session_id:
                storage.append_tool_result(session_id, tool_type, result, round_num=round_num)

            if "error" in result:
                print(f"    {_C.RED}✗ {result['error'][:120]}{_C.RESET} {_C.DIM}({elapsed:.1f}s){_C.RESET}")
            else:
                summary = _summarize_result(result)
                print(f"    {_C.GREEN}✓{_C.RESET} {_C.DIM}{summary} ({elapsed:.1f}s){_C.RESET}")

            result_str = json.dumps(result, ensure_ascii=False, default=str)
            if len(result_str) > 30_000:
                result_str = result_str[:30_000] + "\n... (truncated)"

            messages.append({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": result_str,
            })

    return "(Max tool-call rounds reached. Please continue or rephrase.)", False


async def _continuous_loop(
    adapter: LLMAdapter,
    model: str,
    messages: list[dict[str, Any]],
    tools: list[dict[str, Any]],
    name_map: dict[str, str],
    registry: ToolRegistry,
    cwd: str,
    tracker: TokenTracker,
    temperature: float,
    max_tokens: int,
    storage: Any = None,
    session_id: str = "",
    autonomy: AutonomyConfig | None = None,
    context_window: int | None = None,
) -> None:
    """L3/L4 continuous loop — agent keeps working without waiting for user input.

    After the initial response, the agent is prompted to continue working.
    The loop runs until the agent says it's done, hits an error, or is interrupted (Ctrl+C).
    """
    _CONTINUE_PROMPT = (
        "Continue working on the objective. If you're done with all tasks, "
        "respond with exactly: OBJECTIVE_COMPLETE. If you need user input for a critical decision, "
        "respond with exactly: NEED_INPUT followed by your question."
    )
    _MAX_CONTINUOUS_ROUNDS = 20  # safety limit for continuous mode

    for continuous_round in range(_MAX_CONTINUOUS_ROUNDS):
        # Inject continue prompt
        messages.append({"role": "user", "content": _CONTINUE_PROMPT})

        try:
            response_text, was_streamed = await _agentic_loop(
                adapter=adapter,
                model=model,
                messages=messages,
                tools=tools,
                name_map=name_map,
                registry=registry,
                cwd=cwd,
                tracker=tracker,
                temperature=temperature,
                max_tokens=max_tokens,
                storage=storage,
                session_id=session_id,
                autonomy=autonomy,
                context_window=context_window,
            )
        except KeyboardInterrupt:
            print(f"\n{_C.YELLOW}(Continuous loop interrupted by user){_C.RESET}")
            messages.append({"role": "assistant", "content": "(interrupted by user)"})
            return

        if not response_text:
            return

        messages.append({"role": "assistant", "content": response_text})
        if storage and session_id:
            storage.append_assistant_message(session_id, response_text)

        if not was_streamed:
            print(f"\n{response_text}")

        # Check if agent says it's done
        upper = response_text.upper().strip()
        if "OBJECTIVE_COMPLETE" in upper:
            print(f"\n  {_C.GREEN}Objective completed.{_C.RESET}")
            if storage and session_id:
                storage.create_checkpoint(session_id, message_index=len(messages), label="objective_complete")
            return

        # Check if agent needs input
        if "NEED_INPUT" in upper:
            print(f"\n  {_C.YELLOW}Agent needs your input to continue.{_C.RESET}")
            return

        # Checkpoint every 5 continuous rounds
        if continuous_round > 0 and continuous_round % 5 == 0:
            if storage and session_id:
                storage.create_checkpoint(session_id, message_index=len(messages), label=f"auto_continuous_{continuous_round}")

    print(f"\n  {_C.YELLOW}Continuous loop reached max rounds ({_MAX_CONTINUOUS_ROUNDS}). Pausing.{_C.RESET}")


def _format_args(args: dict[str, Any]) -> str:
    parts: list[str] = []
    for k, v in args.items():
        val = str(v)
        if len(val) > 60:
            val = val[:57] + "..."
        parts.append(f"{k}={val}")
    text = ", ".join(parts)
    return text[:200] if len(text) > 200 else text


def _summarize_result(result: dict[str, Any]) -> str:
    parts: list[str] = []
    for key in ("path", "count", "lines", "files_changed", "exit_code", "branch", "hash"):
        if key in result:
            val = result[key]
            if isinstance(val, str) and len(val) > 50:
                val = val[:47] + "..."
            parts.append(f"{key}={val}")
    if parts:
        return ", ".join(parts[:4])
    return f"keys: {list(result.keys())[:5]}"


# ---------------------------------------------------------------------------
# Context compression
# ---------------------------------------------------------------------------

def _estimate_tokens(messages: list[dict[str, Any]]) -> int:
    total = 0
    for msg in messages:
        content = msg.get("content") or ""
        total += len(content) // 4
        for tc in msg.get("tool_calls", []):
            total += len(tc.get("function", {}).get("arguments", "")) // 4
    return total


def _compact_messages(messages: list[dict[str, Any]], max_tokens: int = 80_000) -> list[dict[str, Any]]:
    est = _estimate_tokens(messages)
    if est <= max_tokens:
        return messages

    keep_tail = 12
    compacted = list(messages)
    for i in range(1, max(1, len(compacted) - keep_tail)):
        msg = compacted[i]
        if msg.get("role") == "tool":
            content = msg.get("content", "")
            if len(content) > 500:
                compacted[i] = {**msg, "content": content[:200] + "\n... (compacted)"}

    est = _estimate_tokens(compacted)
    if est <= max_tokens:
        return compacted

    if len(compacted) > keep_tail + 1:
        summary = {
            "role": "user",
            "content": "(Earlier conversation was compacted to save context. Continue from here.)",
        }
        compacted = [compacted[0], summary] + compacted[-(keep_tail):]

    return compacted


# ---------------------------------------------------------------------------
# Interactive terminal session
# ---------------------------------------------------------------------------

_BANNER = f"""{_C.BOLD}{_C.MAGENTA}
  ╔══════════════════════════════════════╗
  ║         Mirai Code v0.1.0           ║
  ║   Agentic coding in your terminal   ║
  ╚══════════════════════════════════════╝{_C.RESET}
"""


async def run_interactive_session(
    provider: str,
    model: str,
    cwd: str,
    api_key: str = "",
    base_url: str = "",
    temperature: float = 0.3,
    max_tokens: int = 4096,
    context_file: str = "",
    resume_session_id: str = "",
    autonomy_level: str = "copilot",
    context_window: int | None = None,
) -> None:
    """Main interactive session entry point."""
    from datamirai_engine.cli.session_storage import SessionStorage

    print(_BANNER)
    print(f"  {_C.DIM}Provider:{_C.RESET} {_C.BOLD}{provider}{_C.RESET}")
    print(f"  {_C.DIM}Model:{_C.RESET}    {_C.BOLD}{model}{_C.RESET}")
    print(f"  {_C.DIM}CWD:{_C.RESET}      {_C.BOLD}{cwd}{_C.RESET}")
    print()

    adapter = _create_adapter(provider, api_key=api_key, base_url=base_url)
    registry = _create_full_registry()
    # Local models (Ollama) work better with fewer tools
    tool_mode = "core" if provider == "ollama" else "all"
    tool_schemas = build_tool_schemas(registry, mode=tool_mode)
    name_map = get_tool_name_map(registry)
    tracker = TokenTracker()
    storage = SessionStorage()

    autonomy = get_autonomy_config(autonomy_level)

    print(f"  {_C.DIM}Tools loaded: {len(tool_schemas)} across {len({s['function']['name'].split('_')[0] for s in tool_schemas})} categories{_C.RESET}")
    print(f"  {_C.DIM}Autonomy: {_C.BOLD}{autonomy.level}{_C.RESET} {_C.DIM}(max {autonomy.max_tool_rounds} rounds/turn){_C.RESET}")
    if context_window:
        ctx_label = f"{context_window // 1000}K" if context_window < 1_000_000 else f"{context_window // 1_000_000}M"
        print(f"  {_C.DIM}Context window: {_C.BOLD}{ctx_label}{_C.RESET}")

    extra_context = ""
    if context_file and os.path.isfile(context_file):
        with open(context_file, "r") as f:
            extra_context = f"\n# Project context\n{f.read()}"

    system_prompt = build_system_prompt(cwd, autonomy_level=autonomy_level, extra_context=extra_context)

    # Resume or create session
    if resume_session_id:
        manifest = storage.read_manifest(resume_session_id)
        if manifest is None:
            print(f"  {_C.RED}Session {resume_session_id} not found{_C.RESET}")
            return
        messages = storage.rebuild_messages(resume_session_id)
        if not messages or messages[0].get("role") != "system":
            messages.insert(0, {"role": "system", "content": system_prompt})
        session_id = resume_session_id
        print(f"  {_C.GREEN}Resumed session: {session_id}{_C.RESET}")
    else:
        manifest = storage.create_session(provider=provider, model=model, cwd=cwd)
        session_id = manifest.id
        messages: list[dict[str, Any]] = [
            {"role": "system", "content": system_prompt},
        ]
        storage.append_entry(session_id, __import__(
            "datamirai_engine.cli.session_storage", fromlist=["TranscriptEntry"]
        ).TranscriptEntry(ts=time.time(), role="system", content="(session started)"))
        print(f"  {_C.DIM}Session: {session_id}{_C.RESET}")

    print(f"  {_C.DIM}Type /help for commands, Ctrl+C while thinking to interrupt{_C.RESET}")
    print()

    # Status bar at the bottom
    _max_ctx = context_window or 131072
    status_bar = StatusBar(max_ctx=_max_ctx)
    status_bar.setup_scroll_region()

    def _update_status():
        ctx_used = _estimate_tokens(messages)
        status_bar.render(
            ctx_used=ctx_used,
            total_tokens=tracker.total_input + tracker.total_output,
            msg_count=len(messages),
            model=model,
            autonomy=autonomy.level,
        )

    _update_status()

    while True:
        try:
            user_input = input(f"\n{_C.BOLD}{_C.BLUE}>{_C.RESET} ").strip()
        except (KeyboardInterrupt, EOFError):
            status_bar.cleanup()
            storage.close_session(session_id)
            print(f"\n{_C.DIM}Session saved: {session_id}{_C.RESET}")
            print(f"{_C.DIM}Bye! {tracker.summary()}{_C.RESET}")
            break

        if not user_input:
            continue

        if user_input.startswith("/"):
            handled = _handle_slash(user_input, messages, tracker, tool_schemas, storage, session_id)
            if handled == "quit":
                status_bar.cleanup()
                storage.close_session(session_id)
                print(f"{_C.DIM}Session saved: {session_id}{_C.RESET}")
                break
            continue

        # Detect inline assets (images, audio, docs, video)
        from datamirai_engine.cli.multimodal import detect_assets_in_message, build_multimodal_content
        cleaned_text, assets = detect_assets_in_message(user_input, cwd=cwd)
        if assets:
            content = build_multimodal_content(cleaned_text, assets=assets)
            messages.append({"role": "user", "content": content})
            storage.append_user_message(session_id, user_input)
            for a in assets:
                print(f"  {_C.DIM}Attached {a.asset_type}: {os.path.basename(a.path)} ({a.size_bytes:,} bytes){_C.RESET}")
        else:
            messages.append({"role": "user", "content": user_input})
            storage.append_user_message(session_id, user_input)
        messages = _compact_messages(messages)

        try:
            response_text, was_streamed = await _agentic_loop(
                adapter=adapter,
                model=model,
                messages=messages,
                tools=tool_schemas,
                name_map=name_map,
                registry=registry,
                cwd=cwd,
                tracker=tracker,
                temperature=temperature,
                max_tokens=max_tokens,
                storage=storage,
                session_id=session_id,
                autonomy=autonomy,
                context_window=context_window,
            )

            if response_text:
                messages.append({"role": "assistant", "content": response_text})
                storage.append_assistant_message(
                    session_id, response_text,
                    tokens={"in": tracker.total_input, "out": tracker.total_output},
                )
                if not was_streamed:
                    print(f"\n{response_text}")

            _update_status()

            # Auto-checkpoint after each complete turn
            if autonomy.auto_checkpoint:
                storage.create_checkpoint(session_id, message_index=len(messages), label="auto")

            # L3/L4: Continuous loop — agent keeps working without user input
            if autonomy.level in ("autopilot", "self_driving") and response_text:
                await _continuous_loop(
                    adapter=adapter,
                    model=model,
                    messages=messages,
                    tools=tool_schemas,
                    name_map=name_map,
                    registry=registry,
                    cwd=cwd,
                    tracker=tracker,
                    temperature=temperature,
                    max_tokens=max_tokens,
                    storage=storage,
                    session_id=session_id,
                    autonomy=autonomy,
                    context_window=context_window,
                )

        except KeyboardInterrupt:
            print(f"\n{_C.YELLOW}(Interrupted — LLM call cancelled){_C.RESET}")
            messages.append({"role": "assistant", "content": "(interrupted by user)"})
            storage.append_assistant_message(session_id, "(interrupted by user)")
        except Exception as exc:
            err_type = type(exc).__name__
            err_msg = str(exc) or "(empty error)"
            if "timeout" in err_msg.lower() or "ReadTimeout" in err_type:
                print(f"\n{_C.RED}Error: Model timed out. Try a smaller model or reduce context window.{_C.RESET}")
                print(f"  {_C.DIM}{err_type}: {err_msg[:200]}{_C.RESET}")
            elif "connect" in err_msg.lower() or "refused" in err_msg.lower():
                print(f"\n{_C.RED}Error: Cannot connect to Ollama. Is it running?{_C.RESET}")
                print(f"  {_C.DIM}{err_type}: {err_msg[:200]}{_C.RESET}")
            else:
                print(f"\n{_C.RED}Error ({err_type}): {err_msg[:300]}{_C.RESET}")
            messages.append({"role": "assistant", "content": f"(error: {err_type}: {err_msg})"})
            storage.append_assistant_message(session_id, f"(error: {err_type}: {err_msg})")


def _handle_slash(
    cmd: str,
    messages: list[dict[str, Any]],
    tracker: TokenTracker,
    tool_schemas: list[dict[str, Any]],
    storage: Any = None,
    session_id: str = "",
) -> str | None:
    parts = cmd.split(None, 1)
    command = parts[0].lower()
    arg = parts[1].strip() if len(parts) > 1 else ""

    if command in ("/quit", "/exit", "/q"):
        print(f"{_C.DIM}Bye! {tracker.summary()}{_C.RESET}")
        return "quit"

    if command == "/clear":
        system_msg = messages[0] if messages else None
        messages.clear()
        if system_msg:
            messages.append(system_msg)
        print(f"{_C.GREEN}Context cleared.{_C.RESET}")
        return None

    if command == "/compact":
        before = _estimate_tokens(messages)
        compacted = _compact_messages(messages, max_tokens=40_000)
        messages.clear()
        messages.extend(compacted)
        after = _estimate_tokens(messages)
        print(f"{_C.GREEN}Compacted: ~{before:,} → ~{after:,} tokens{_C.RESET}")
        return None

    if command == "/tokens":
        est = _estimate_tokens(messages)
        print(f"{_C.DIM}Session: {tracker.summary()}{_C.RESET}")
        print(f"{_C.DIM}Context: ~{est:,} tokens in {len(messages)} messages{_C.RESET}")
        return None

    if command == "/tools":
        cats: dict[str, list[str]] = {}
        for s in tool_schemas:
            name = s["function"]["name"]
            cat = name.split("_")[0]
            cats.setdefault(cat, []).append(name)
        for cat, tools in sorted(cats.items()):
            print(f"  {_C.BOLD}{cat}/{_C.RESET} ({len(tools)})")
            for t in sorted(tools):
                print(f"    {_C.DIM}{t}{_C.RESET}")
        return None

    # --- Session commands ---

    if command == "/session":
        print(f"  {_C.DIM}ID: {session_id}{_C.RESET}")
        if storage:
            m = storage.read_manifest(session_id)
            if m:
                import datetime
                created = datetime.datetime.fromtimestamp(m.created_at).strftime("%Y-%m-%d %H:%M")
                print(f"  {_C.DIM}Created: {created}{_C.RESET}")
                print(f"  {_C.DIM}Messages: {m.message_count} | Checkpoints: {m.checkpoint_count}{_C.RESET}")
        return None

    if command == "/sessions":
        if not storage:
            print(f"{_C.YELLOW}No storage configured{_C.RESET}")
            return None
        sessions = storage.list_sessions(limit=15)
        if not sessions:
            print(f"  {_C.DIM}No saved sessions{_C.RESET}")
            return None
        import datetime
        for s in sessions:
            dt = datetime.datetime.fromtimestamp(s.updated_at).strftime("%m-%d %H:%M")
            status_icon = "●" if s.status == "active" else "○"
            current = " ← current" if s.id == session_id else ""
            print(f"  {status_icon} {_C.BOLD}{s.id}{_C.RESET} {_C.DIM}{dt} | {s.provider}/{s.model} | {s.message_count} msgs{current}{_C.RESET}")
        return None

    if command == "/checkpoint":
        if not storage:
            print(f"{_C.YELLOW}No storage configured{_C.RESET}")
            return None
        label = arg or "manual"
        cp = storage.create_checkpoint(session_id, message_index=len(messages), label=label)
        print(f"  {_C.GREEN}Checkpoint created: {cp.id} ({label}) at message {cp.message_index}{_C.RESET}")
        return None

    if command == "/checkpoints":
        if not storage:
            print(f"{_C.YELLOW}No storage configured{_C.RESET}")
            return None
        cps = storage.list_checkpoints(session_id)
        if not cps:
            print(f"  {_C.DIM}No checkpoints{_C.RESET}")
            return None
        import datetime
        for cp in cps:
            dt = datetime.datetime.fromtimestamp(cp.ts).strftime("%H:%M:%S")
            print(f"  {_C.DIM}{cp.id} | {dt} | msg #{cp.message_index} | {cp.label}{_C.RESET}")
        return None

    if command == "/rollback":
        if not storage or not arg:
            print(f"{_C.YELLOW}Usage: /rollback <checkpoint_id>{_C.RESET}")
            return None
        rebuilt = storage.rollback_to_checkpoint(session_id, arg)
        if rebuilt is None:
            print(f"{_C.RED}Checkpoint {arg} not found{_C.RESET}")
            return None
        # Rebuild messages in-place
        system_msg = messages[0] if messages and messages[0].get("role") == "system" else None
        messages.clear()
        if system_msg:
            messages.append(system_msg)
        messages.extend(rebuilt)
        print(f"{_C.GREEN}Rolled back to {arg}. Messages: {len(messages)}{_C.RESET}")
        return None

    if command == "/help":
        print(f"""
{_C.BOLD}Commands:{_C.RESET}
  {_C.BOLD}Session:{_C.RESET}
    /session       — Current session info
    /sessions      — List all saved sessions
    /checkpoint    — Create a checkpoint (optional label: /checkpoint before_refactor)
    /checkpoints   — List checkpoints in current session
    /rollback <id> — Rollback to a checkpoint

  {_C.BOLD}Context:{_C.RESET}
    /clear         — Reset conversation (keep system prompt)
    /compact       — Force context compression
    /tokens        — Show token usage stats
    /tools         — List all available tools

  {_C.BOLD}Other:{_C.RESET}
    /quit          — Save session and exit
    /help          — This message

  {_C.BOLD}Tip:{_C.RESET} Press Ctrl+C while the LLM is thinking to interrupt.
""")
        return None

    print(f"{_C.YELLOW}Unknown command: {command}. Type /help{_C.RESET}")
    return None


# ---------------------------------------------------------------------------
# Entry point (called from main.py or directly)
# ---------------------------------------------------------------------------

def start_interactive_terminal(
    provider: str = "groq",
    model: str = "",
    cwd: str = ".",
    api_key: str = "",
    base_url: str = "",
    temperature: float = 0.3,
    max_tokens: int = 4096,
    context_file: str = "",
    resume: str = "",
    autonomy_level: str = "copilot",
    context_window: int | None = None,
) -> None:
    """Synchronous entry point for the interactive agent terminal."""
    default_models = {
        "groq": "qwen-qwq-32b",
        "nvidia": "meta/llama-3.3-70b-instruct",
        "ollama": "qwen3:32b",
        "openai": "gpt-4o",
        "openrouter": "meta-llama/llama-3.3-70b-instruct",
    }
    if not model:
        model = default_models.get(provider, "llama-3.3-70b-versatile")

    cwd = os.path.abspath(cwd)

    try:
        asyncio.run(
            run_interactive_session(
                provider=provider,
                model=model,
                cwd=cwd,
                api_key=api_key,
                base_url=base_url,
                temperature=temperature,
                max_tokens=max_tokens,
                context_file=context_file,
                resume_session_id=resume,
                autonomy_level=autonomy_level,
                context_window=context_window,
            )
        )
    except KeyboardInterrupt:
        print(f"\n{_C.DIM}Bye!{_C.RESET}")
