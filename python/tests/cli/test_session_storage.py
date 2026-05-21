"""Tests for session_storage — JSONL persistence, checkpoints, rollback."""

from __future__ import annotations

import json
import os
import time

import pytest

from datamirai_engine.cli.session_storage import (
    Checkpoint,
    SessionManifest,
    SessionStorage,
    TranscriptEntry,
)


@pytest.fixture
def storage(tmp_path):
    """SessionStorage backed by a temp directory."""
    return SessionStorage(base_dir=str(tmp_path))


class TestSessionManifest:
    def test_roundtrip(self):
        m = SessionManifest(
            id="ses_123", provider="groq", model="qwen", cwd="/tmp", created_at=1.0
        )
        d = m.to_dict()
        m2 = SessionManifest.from_dict(d)
        assert m2.id == "ses_123"
        assert m2.provider == "groq"
        assert m2.status == "active"

    def test_from_dict_ignores_extra_keys(self):
        d = {"id": "x", "provider": "p", "model": "m", "cwd": "/", "created_at": 1.0, "extra": "ignored"}
        m = SessionManifest.from_dict(d)
        assert m.id == "x"


class TestTranscriptEntry:
    def test_json_roundtrip(self):
        e = TranscriptEntry(ts=123.456, role="user", content="hello")
        line = e.to_json_line()
        e2 = TranscriptEntry.from_json_line(line)
        assert e2.role == "user"
        assert e2.content == "hello"
        assert e2.ts == 123.456

    def test_metadata_included(self):
        e = TranscriptEntry(ts=1.0, role="tool_call", metadata={"tool": "bash", "args": {"cmd": "ls"}})
        line = e.to_json_line()
        parsed = json.loads(line)
        assert parsed["tool"] == "bash"
        assert parsed["args"]["cmd"] == "ls"

    def test_empty_content_not_included(self):
        e = TranscriptEntry(ts=1.0, role="checkpoint")
        line = e.to_json_line()
        parsed = json.loads(line)
        assert "content" not in parsed


class TestSessionCreate:
    def test_create_session(self, storage):
        manifest = storage.create_session(provider="ollama", model="qwen3:8b", cwd="/tmp")
        assert manifest.id.startswith("ses_")
        assert manifest.provider == "ollama"
        assert manifest.model == "qwen3:8b"
        assert manifest.status == "active"

        # Verify files on disk
        session_dir = storage._session_dir(manifest.id)
        assert os.path.isdir(session_dir)
        assert os.path.isfile(storage._manifest_path(manifest.id))
        assert os.path.isfile(storage._transcript_path(manifest.id))

    def test_read_manifest(self, storage):
        m = storage.create_session(provider="groq", model="llama", cwd="/tmp")
        m2 = storage.read_manifest(m.id)
        assert m2 is not None
        assert m2.id == m.id
        assert m2.provider == "groq"

    def test_read_manifest_nonexistent(self, storage):
        assert storage.read_manifest("fake_id") is None


class TestAppendAndRead:
    def test_append_user_message(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_user_message(m.id, "hello world")

        entries = storage.read_transcript(m.id)
        assert len(entries) == 1
        assert entries[0].role == "user"
        assert entries[0].content == "hello world"

    def test_append_assistant_message_with_tokens(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_assistant_message(m.id, "hi there", tokens={"input": 10, "output": 5})

        entries = storage.read_transcript(m.id)
        assert len(entries) == 1
        assert entries[0].role == "assistant"
        assert entries[0].metadata.get("tokens") == {"input": 10, "output": 5}

    def test_append_tool_call(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_tool_call(m.id, "filesystem/read_file", {"path": "/tmp/x.py"}, round_num=1)

        entries = storage.read_transcript(m.id)
        assert len(entries) == 1
        assert entries[0].role == "tool_call"
        assert entries[0].metadata["tool"] == "filesystem/read_file"
        assert entries[0].metadata["round"] == 1

    def test_append_tool_result(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_tool_result(m.id, "system/bash", {"stdout": "ok", "exit_code": 0})

        entries = storage.read_transcript(m.id)
        assert len(entries) == 1
        assert entries[0].role == "tool_result"
        assert "ok" in entries[0].content

    def test_multiple_entries_order(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_user_message(m.id, "first")
        storage.append_assistant_message(m.id, "second")
        storage.append_user_message(m.id, "third")

        entries = storage.read_transcript(m.id)
        assert len(entries) == 3
        assert [e.role for e in entries] == ["user", "assistant", "user"]
        assert [e.content for e in entries] == ["first", "second", "third"]

    def test_manifest_updates_on_append(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        old_ts = m.updated_at
        time.sleep(0.01)
        storage.append_user_message(m.id, "hello")
        m2 = storage.read_manifest(m.id)
        assert m2.updated_at >= old_ts
        assert m2.message_count == 1


class TestRebuildMessages:
    def test_rebuild_basic(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_entry(m.id, TranscriptEntry(ts=1.0, role="system", content="you are helpful"))
        storage.append_user_message(m.id, "hello")
        storage.append_tool_call(m.id, "bash", {"cmd": "ls"})
        storage.append_tool_result(m.id, "bash", {"stdout": "file.py"})
        storage.append_assistant_message(m.id, "I see file.py")

        messages = storage.rebuild_messages(m.id)
        assert len(messages) == 3  # system, user, assistant (tool events excluded)
        assert messages[0]["role"] == "system"
        assert messages[1]["role"] == "user"
        assert messages[2]["role"] == "assistant"


class TestCheckpoints:
    def test_create_checkpoint(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_user_message(m.id, "hello")
        storage.append_assistant_message(m.id, "hi")

        cp = storage.create_checkpoint(m.id, message_index=2, label="before_edit")
        assert cp.id.startswith("chk_")
        assert cp.label == "before_edit"
        assert cp.message_index == 2

    def test_list_checkpoints(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_user_message(m.id, "a")
        storage.create_checkpoint(m.id, message_index=1, label="cp1")
        storage.append_user_message(m.id, "b")
        storage.create_checkpoint(m.id, message_index=3, label="cp2")

        cps = storage.list_checkpoints(m.id)
        assert len(cps) == 2
        assert cps[0].label == "cp1"
        assert cps[1].label == "cp2"

    def test_manifest_checkpoint_count(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.create_checkpoint(m.id, message_index=0)
        storage.create_checkpoint(m.id, message_index=0)
        m2 = storage.read_manifest(m.id)
        assert m2.checkpoint_count == 2


class TestRollback:
    def test_rollback_to_checkpoint(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.append_user_message(m.id, "first")
        storage.append_assistant_message(m.id, "reply1")
        cp = storage.create_checkpoint(m.id, message_index=2, label="safe_point")
        storage.append_user_message(m.id, "second")
        storage.append_assistant_message(m.id, "reply2")

        # Before rollback: 4 real entries + 1 checkpoint = 5
        entries_before = storage.read_transcript(m.id)
        assert len(entries_before) == 5

        messages = storage.rollback_to_checkpoint(m.id, cp.id)
        assert messages is not None
        assert len(messages) == 2  # user + assistant (before checkpoint)

        # After rollback: only 3 entries (user, assistant, checkpoint)
        entries_after = storage.read_transcript(m.id)
        assert len(entries_after) == 3

    def test_rollback_nonexistent_checkpoint(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        result = storage.rollback_to_checkpoint(m.id, "fake_chk")
        assert result is None


class TestListSessions:
    def test_list_empty(self, storage):
        assert storage.list_sessions() == []

    def test_list_multiple(self, storage):
        storage.create_session(provider="a", model="m1", cwd="/")
        time.sleep(0.01)
        storage.create_session(provider="b", model="m2", cwd="/")

        sessions = storage.list_sessions()
        assert len(sessions) == 2
        # Newest first
        assert sessions[0].provider == "b"

    def test_list_limit(self, storage):
        for i in range(5):
            storage.create_session(provider=f"p{i}", model="m", cwd="/")
            time.sleep(0.01)
        assert len(storage.list_sessions(limit=3)) == 3


class TestCloseSession:
    def test_close(self, storage):
        m = storage.create_session(provider="p", model="m", cwd="/")
        storage.close_session(m.id)
        m2 = storage.read_manifest(m.id)
        assert m2.status == "closed"
