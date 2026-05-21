"""session_storage — persist conversations as JSONL with checkpoints.

Each session lives in ~/.datamirai/sessions/<session_id>/ with:
- manifest.json  — metadata (id, provider, model, cwd, created_at, etc.)
- transcript.jsonl — append-only log of every event in the conversation
"""

from __future__ import annotations

import json
import os
import time
import uuid
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any


# ---------------------------------------------------------------------------
# Data models
# ---------------------------------------------------------------------------

@dataclass
class SessionManifest:
    id: str
    provider: str
    model: str
    cwd: str
    created_at: float
    updated_at: float = 0.0
    message_count: int = 0
    checkpoint_count: int = 0
    status: str = "active"  # active | closed

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> SessionManifest:
        return cls(**{k: v for k, v in d.items() if k in cls.__dataclass_fields__})


@dataclass
class TranscriptEntry:
    ts: float
    role: str  # user | assistant | thinking | tool_call | tool_result | checkpoint | system
    content: str = ""
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_json_line(self) -> str:
        d: dict[str, Any] = {"ts": self.ts, "role": self.role}
        if self.content:
            d["content"] = self.content
        if self.metadata:
            d.update(self.metadata)
        return json.dumps(d, ensure_ascii=False, default=str)

    @classmethod
    def from_json_line(cls, line: str) -> TranscriptEntry:
        d = json.loads(line)
        ts = d.pop("ts", 0.0)
        role = d.pop("role", "system")
        content = d.pop("content", "")
        return cls(ts=ts, role=role, content=content, metadata=d)


@dataclass
class Checkpoint:
    id: str
    label: str
    message_index: int  # index into the messages list
    ts: float


# ---------------------------------------------------------------------------
# Session storage
# ---------------------------------------------------------------------------

_DEFAULT_BASE = os.path.expanduser("~/.datamirai/sessions")


class SessionStorage:
    """Manages session persistence on disk."""

    def __init__(self, base_dir: str = _DEFAULT_BASE) -> None:
        self.base_dir = base_dir

    def _session_dir(self, session_id: str) -> str:
        return os.path.join(self.base_dir, session_id)

    def _manifest_path(self, session_id: str) -> str:
        return os.path.join(self._session_dir(session_id), "manifest.json")

    def _transcript_path(self, session_id: str) -> str:
        return os.path.join(self._session_dir(session_id), "transcript.jsonl")

    # --- Create ---

    def create_session(
        self, *, provider: str, model: str, cwd: str
    ) -> SessionManifest:
        """Create a new session directory with manifest."""
        session_id = f"ses_{int(time.time())}_{uuid.uuid4().hex[:8]}"
        session_dir = self._session_dir(session_id)
        os.makedirs(session_dir, exist_ok=True)

        manifest = SessionManifest(
            id=session_id,
            provider=provider,
            model=model,
            cwd=cwd,
            created_at=time.time(),
            updated_at=time.time(),
        )

        with open(self._manifest_path(session_id), "w") as f:
            json.dump(manifest.to_dict(), f, indent=2)

        # Create empty transcript
        Path(self._transcript_path(session_id)).touch()

        return manifest

    # --- Append ---

    def append_entry(self, session_id: str, entry: TranscriptEntry) -> None:
        """Append a single entry to the transcript."""
        path = self._transcript_path(session_id)
        with open(path, "a", encoding="utf-8") as f:
            f.write(entry.to_json_line() + "\n")

        self._update_manifest_ts(session_id)

    def append_user_message(self, session_id: str, content: str) -> None:
        self.append_entry(session_id, TranscriptEntry(
            ts=time.time(), role="user", content=content,
        ))

    def append_assistant_message(
        self, session_id: str, content: str, tokens: dict[str, int] | None = None
    ) -> None:
        meta = {"tokens": tokens} if tokens else {}
        self.append_entry(session_id, TranscriptEntry(
            ts=time.time(), role="assistant", content=content, metadata=meta,
        ))

    def append_tool_call(
        self, session_id: str, tool: str, args: dict[str, Any], round_num: int = 0
    ) -> None:
        self.append_entry(session_id, TranscriptEntry(
            ts=time.time(), role="tool_call", metadata={"tool": tool, "args": args, "round": round_num},
        ))

    def append_tool_result(
        self, session_id: str, tool: str, result: dict[str, Any], round_num: int = 0
    ) -> None:
        result_str = json.dumps(result, ensure_ascii=False, default=str)
        if len(result_str) > 5000:
            result_str = result_str[:5000] + "...(truncated)"
        self.append_entry(session_id, TranscriptEntry(
            ts=time.time(), role="tool_result",
            content=result_str, metadata={"tool": tool, "round": round_num},
        ))

    # --- Checkpoints ---

    def create_checkpoint(
        self, session_id: str, message_index: int, label: str = "auto"
    ) -> Checkpoint:
        """Create a checkpoint at the given message index."""
        cp = Checkpoint(
            id=f"chk_{uuid.uuid4().hex[:6]}",
            label=label,
            message_index=message_index,
            ts=time.time(),
        )
        self.append_entry(session_id, TranscriptEntry(
            ts=cp.ts, role="checkpoint",
            metadata={"checkpoint_id": cp.id, "label": cp.label, "message_index": cp.message_index},
        ))
        self._increment_checkpoint_count(session_id)
        return cp

    def list_checkpoints(self, session_id: str) -> list[Checkpoint]:
        """List all checkpoints in a session."""
        entries = self.read_transcript(session_id)
        checkpoints: list[Checkpoint] = []
        for e in entries:
            if e.role == "checkpoint":
                checkpoints.append(Checkpoint(
                    id=e.metadata.get("checkpoint_id", ""),
                    label=e.metadata.get("label", ""),
                    message_index=e.metadata.get("message_index", 0),
                    ts=e.ts,
                ))
        return checkpoints

    # --- Read ---

    def read_manifest(self, session_id: str) -> SessionManifest | None:
        """Read session manifest. Returns None if not found."""
        path = self._manifest_path(session_id)
        if not os.path.isfile(path):
            return None
        with open(path) as f:
            return SessionManifest.from_dict(json.load(f))

    def read_transcript(self, session_id: str) -> list[TranscriptEntry]:
        """Read all transcript entries."""
        path = self._transcript_path(session_id)
        if not os.path.isfile(path):
            return []
        entries: list[TranscriptEntry] = []
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    entries.append(TranscriptEntry.from_json_line(line))
        return entries

    def rebuild_messages(self, session_id: str) -> list[dict[str, Any]]:
        """Rebuild the LLM messages array from the transcript.

        Only includes user and assistant messages (not tool_call/tool_result
        which are embedded in the assistant turn's tool_calls).
        """
        entries = self.read_transcript(session_id)
        messages: list[dict[str, Any]] = []
        for e in entries:
            if e.role == "user":
                messages.append({"role": "user", "content": e.content})
            elif e.role == "assistant":
                messages.append({"role": "assistant", "content": e.content})
            elif e.role == "system":
                messages.append({"role": "system", "content": e.content})
        return messages

    # --- List sessions ---

    def list_sessions(self, limit: int = 20) -> list[SessionManifest]:
        """List all sessions, newest first."""
        if not os.path.isdir(self.base_dir):
            return []
        sessions: list[SessionManifest] = []
        for name in os.listdir(self.base_dir):
            manifest = self.read_manifest(name)
            if manifest:
                sessions.append(manifest)
        sessions.sort(key=lambda s: s.updated_at, reverse=True)
        return sessions[:limit]

    # --- Close ---

    def close_session(self, session_id: str) -> None:
        """Mark session as closed."""
        path = self._manifest_path(session_id)
        if not os.path.isfile(path):
            return
        with open(path) as f:
            data = json.load(f)
        data["status"] = "closed"
        data["updated_at"] = time.time()
        with open(path, "w") as f:
            json.dump(data, f, indent=2)

    # --- Rollback ---

    def rollback_to_checkpoint(
        self, session_id: str, checkpoint_id: str
    ) -> list[dict[str, Any]] | None:
        """Rollback transcript to a checkpoint. Returns rebuilt messages or None if not found."""
        checkpoints = self.list_checkpoints(session_id)
        target = None
        for cp in checkpoints:
            if cp.id == checkpoint_id:
                target = cp
                break
        if target is None:
            return None

        # Read all entries, keep only up to checkpoint's message_index
        entries = self.read_transcript(session_id)
        # Find the checkpoint entry itself
        keep_until = 0
        for i, e in enumerate(entries):
            if e.role == "checkpoint" and e.metadata.get("checkpoint_id") == checkpoint_id:
                keep_until = i + 1
                break

        if keep_until == 0:
            return None

        # Rewrite transcript
        kept = entries[:keep_until]
        path = self._transcript_path(session_id)
        with open(path, "w", encoding="utf-8") as f:
            for entry in kept:
                f.write(entry.to_json_line() + "\n")

        return self.rebuild_messages(session_id)

    # --- Internal helpers ---

    def _update_manifest_ts(self, session_id: str) -> None:
        path = self._manifest_path(session_id)
        if not os.path.isfile(path):
            return
        with open(path) as f:
            data = json.load(f)
        data["updated_at"] = time.time()
        data["message_count"] = data.get("message_count", 0) + 1
        with open(path, "w") as f:
            json.dump(data, f, indent=2)

    def _increment_checkpoint_count(self, session_id: str) -> None:
        path = self._manifest_path(session_id)
        if not os.path.isfile(path):
            return
        with open(path) as f:
            data = json.load(f)
        data["checkpoint_count"] = data.get("checkpoint_count", 0) + 1
        with open(path, "w") as f:
            json.dump(data, f, indent=2)
