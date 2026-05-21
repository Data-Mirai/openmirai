"""E2E tests — simulate a real user session with a mock LLM.

These tests exercise the full lifecycle:
1. Create session → talk → tools execute → session saved to disk
2. Resume session → continue conversation
3. Checkpoints → rollback → verify state
4. Interrupt handling
"""

from __future__ import annotations

import asyncio
import json
import os
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from datamirai_engine.cli.interactive_agent_terminal import (
    _agentic_loop,
    _create_full_registry,
    TokenTracker,
)
from datamirai_engine.cli.session_storage import SessionStorage
from datamirai_engine.cli.tool_schema_builder import build_tool_schemas, get_tool_name_map
from datamirai_engine.llm.adapter import NormalizedResponse, ToolCall


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def storage(tmp_path):
    return SessionStorage(base_dir=str(tmp_path))


@pytest.fixture
def registry():
    return _create_full_registry()


@pytest.fixture
def tool_schemas(registry):
    return build_tool_schemas(registry)


@pytest.fixture
def name_map(registry):
    return get_tool_name_map(registry)


@pytest.fixture
def tracker():
    return TokenTracker()


def _make_text_response(text: str) -> NormalizedResponse:
    """Mock LLM response with just text (no tool calls)."""
    return NormalizedResponse(
        response=text,
        tokens_used={"input": 50, "output": 20},
        model="test-model",
        provider="test",
    )


def _make_tool_response(tool_name: str, args: dict) -> NormalizedResponse:
    """Mock LLM response that calls a tool."""
    return NormalizedResponse(
        response="",
        tokens_used={"input": 50, "output": 20},
        model="test-model",
        provider="test",
        tool_calls=[
            ToolCall(id="call_1", name=tool_name, arguments=json.dumps(args)),
        ],
    )


# ---------------------------------------------------------------------------
# E2E: Full conversation lifecycle
# ---------------------------------------------------------------------------

class TestFullConversationLifecycle:
    """Simulate: user creates session, sends messages, session is persisted."""

    @pytest.mark.asyncio
    async def test_text_only_conversation_saved(self, storage, registry, tool_schemas, name_map, tracker):
        """User says 'hola' → LLM responds 'hello' → session is saved to disk."""
        session = storage.create_session(provider="test", model="test-model", cwd="/tmp")

        mock_adapter = MagicMock()
        mock_adapter.call_with_messages = AsyncMock(return_value=_make_text_response("Hello! How can I help?"))

        messages = [{"role": "system", "content": "You are helpful."}]
        messages.append({"role": "user", "content": "hola"})
        storage.append_user_message(session.id, "hola")

        response = await _agentic_loop(
            adapter=mock_adapter,
            model="test-model",
            messages=messages,
            tools=tool_schemas,
            name_map=name_map,
            registry=registry,
            cwd="/tmp",
            tracker=tracker,
            temperature=0.3,
            max_tokens=4096,
            storage=storage,
            session_id=session.id,
        )

        # LLM returned text
        response_text, _ = response
        assert response_text == "Hello! How can I help?"
        storage.append_assistant_message(session.id, response_text)

        # Verify transcript on disk
        entries = storage.read_transcript(session.id)
        assert len(entries) == 2  # user + assistant
        assert entries[0].role == "user"
        assert entries[0].content == "hola"
        assert entries[1].role == "assistant"
        assert entries[1].content == "Hello! How can I help?"

    @pytest.mark.asyncio
    async def test_tool_call_conversation_saved(self, storage, registry, tool_schemas, name_map, tracker, tmp_path):
        """User asks to list files → LLM calls tool → result saved."""
        session = storage.create_session(provider="test", model="test-model", cwd=str(tmp_path))

        # Create a test file
        (tmp_path / "test.py").write_text("print('hello')")

        # LLM first calls list_dir, then responds with text
        mock_adapter = MagicMock()
        mock_adapter.call_with_messages = AsyncMock(side_effect=[
            _make_tool_response("filesystem_list_dir", {"path": str(tmp_path)}),
            _make_text_response("I see test.py in the directory."),
        ])

        messages = [{"role": "system", "content": "You are helpful."}]
        messages.append({"role": "user", "content": "what files are here?"})
        storage.append_user_message(session.id, "what files are here?")

        response_text, _ = await _agentic_loop(
            adapter=mock_adapter,
            model="test-model",
            messages=messages,
            tools=tool_schemas,
            name_map=name_map,
            registry=registry,
            cwd=str(tmp_path),
            tracker=tracker,
            temperature=0.3,
            max_tokens=4096,
            storage=storage,
            session_id=session.id,
        )

        assert "test.py" in response_text

        # Verify tool call + result were logged
        entries = storage.read_transcript(session.id)
        roles = [e.role for e in entries]
        assert "user" in roles
        assert "tool_call" in roles
        assert "tool_result" in roles

    @pytest.mark.asyncio
    async def test_multiple_tool_calls_in_sequence(self, storage, registry, tool_schemas, name_map, tracker, tmp_path):
        """LLM calls multiple tools before giving final answer."""
        session = storage.create_session(provider="test", model="test-model", cwd=str(tmp_path))
        (tmp_path / "app.py").write_text("# main app")

        mock_adapter = MagicMock()
        mock_adapter.call_with_messages = AsyncMock(side_effect=[
            _make_tool_response("filesystem_list_dir", {"path": str(tmp_path)}),
            _make_tool_response("filesystem_read_file", {"path": str(tmp_path / "app.py")}),
            _make_text_response("The app.py file contains a comment: # main app"),
        ])

        messages = [{"role": "system", "content": "."}, {"role": "user", "content": "read app.py"}]
        storage.append_user_message(session.id, "read app.py")

        response_text, _ = await _agentic_loop(
            adapter=mock_adapter, model="m", messages=messages,
            tools=tool_schemas, name_map=name_map, registry=registry,
            cwd=str(tmp_path), tracker=tracker, temperature=0.3, max_tokens=4096,
            storage=storage, session_id=session.id,
        )

        assert "main app" in response_text

        # Should have 2 tool_calls + 2 tool_results + 1 user = 5 entries
        entries = storage.read_transcript(session.id)
        tool_calls = [e for e in entries if e.role == "tool_call"]
        tool_results = [e for e in entries if e.role == "tool_result"]
        assert len(tool_calls) == 2
        assert len(tool_results) == 2


# ---------------------------------------------------------------------------
# E2E: Session resume
# ---------------------------------------------------------------------------

class TestSessionResume:
    def test_resume_rebuilds_messages(self, storage):
        """Create a session with history, then rebuild messages from disk."""
        session = storage.create_session(provider="test", model="m", cwd="/tmp")

        from datamirai_engine.cli.session_storage import TranscriptEntry
        import time

        storage.append_entry(session.id, TranscriptEntry(ts=time.time(), role="system", content="be helpful"))
        storage.append_user_message(session.id, "hello")
        storage.append_assistant_message(session.id, "hi there")
        storage.append_user_message(session.id, "what time is it?")
        storage.append_assistant_message(session.id, "I don't know the time")

        # Simulate resume: rebuild messages
        messages = storage.rebuild_messages(session.id)
        assert len(messages) == 5  # system + 2 user + 2 assistant
        assert messages[0]["role"] == "system"
        assert messages[1]["content"] == "hello"
        assert messages[4]["content"] == "I don't know the time"


# ---------------------------------------------------------------------------
# E2E: Checkpoints and rollback
# ---------------------------------------------------------------------------

class TestCheckpointRollbackE2E:
    def test_full_checkpoint_rollback_cycle(self, storage):
        """Create session → add messages → checkpoint → add more → rollback → verify."""
        session = storage.create_session(provider="test", model="m", cwd="/tmp")

        # Phase 1: initial conversation
        storage.append_user_message(session.id, "create a file")
        storage.append_assistant_message(session.id, "done, created foo.py")

        # Checkpoint after phase 1
        cp = storage.create_checkpoint(session.id, message_index=2, label="after_create")

        # Phase 2: more conversation (we might want to undo this)
        storage.append_user_message(session.id, "delete everything")
        storage.append_assistant_message(session.id, "deleted all files")

        # Verify we have 5 entries (2 user + 2 assistant + 1 checkpoint)
        entries_before = storage.read_transcript(session.id)
        assert len(entries_before) == 5

        # Rollback to checkpoint
        messages = storage.rollback_to_checkpoint(session.id, cp.id)
        assert messages is not None

        # After rollback: only messages before checkpoint
        entries_after = storage.read_transcript(session.id)
        assert len(entries_after) == 3  # user + assistant + checkpoint

        # Messages rebuilt should be the first 2
        assert len(messages) == 2
        assert messages[0]["content"] == "create a file"
        assert messages[1]["content"] == "done, created foo.py"

    def test_multiple_checkpoints_rollback_to_earliest(self, storage):
        """Create 3 checkpoints, rollback to the first one."""
        session = storage.create_session(provider="test", model="m", cwd="/tmp")

        storage.append_user_message(session.id, "step 1")
        storage.append_assistant_message(session.id, "done 1")
        cp1 = storage.create_checkpoint(session.id, message_index=2, label="cp1")

        storage.append_user_message(session.id, "step 2")
        storage.append_assistant_message(session.id, "done 2")
        cp2 = storage.create_checkpoint(session.id, message_index=5, label="cp2")

        storage.append_user_message(session.id, "step 3")
        storage.append_assistant_message(session.id, "done 3")
        cp3 = storage.create_checkpoint(session.id, message_index=8, label="cp3")

        # Rollback to cp1 (earliest)
        messages = storage.rollback_to_checkpoint(session.id, cp1.id)
        assert messages is not None
        assert len(messages) == 2  # only step 1 messages
        assert messages[0]["content"] == "step 1"

        # Verify only 3 entries remain (user + assistant + checkpoint)
        entries = storage.read_transcript(session.id)
        assert len(entries) == 3


# ---------------------------------------------------------------------------
# E2E: Token tracking
# ---------------------------------------------------------------------------

class TestTokenTracking:
    @pytest.mark.asyncio
    async def test_tokens_accumulated_across_rounds(self, registry, tool_schemas, name_map, tmp_path):
        """Verify tokens are tracked across multiple LLM calls."""
        tracker = TokenTracker()

        mock_adapter = MagicMock()
        mock_adapter.call_with_messages = AsyncMock(side_effect=[
            NormalizedResponse(response="", tokens_used={"input": 100, "output": 50}, model="m", provider="p",
                             tool_calls=[ToolCall(id="c1", name="system_bash", arguments='{"command":"echo hi"}')]),
            NormalizedResponse(response="Done!", tokens_used={"input": 200, "output": 30}, model="m", provider="p"),
        ])

        messages = [{"role": "system", "content": "."}, {"role": "user", "content": "run echo"}]

        await _agentic_loop(
            adapter=mock_adapter, model="m", messages=messages,
            tools=tool_schemas, name_map=name_map, registry=registry,
            cwd=str(tmp_path), tracker=tracker, temperature=0.3, max_tokens=4096,
        )

        assert tracker.total_input == 300  # 100 + 200
        assert tracker.total_output == 80  # 50 + 30
        assert tracker.calls == 2


# ---------------------------------------------------------------------------
# E2E: Session listing
# ---------------------------------------------------------------------------

class TestSessionListing:
    def test_list_shows_all_sessions(self, storage):
        """Multiple sessions created → all appear in listing."""
        import time
        s1 = storage.create_session(provider="ollama", model="qwen3:8b", cwd="/project1")
        time.sleep(0.01)
        s2 = storage.create_session(provider="groq", model="llama-3.3", cwd="/project2")

        sessions = storage.list_sessions()
        assert len(sessions) == 2
        # Newest first
        assert sessions[0].id == s2.id
        assert sessions[1].id == s1.id

    def test_close_session_changes_status(self, storage):
        s = storage.create_session(provider="p", model="m", cwd="/")
        storage.close_session(s.id)
        m = storage.read_manifest(s.id)
        assert m.status == "closed"
