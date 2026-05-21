"""Tests for SharedState."""

import threading

import pytest

from datamirai_engine.core.state import SharedState


class TestSharedState:
    def test_set_and_get(self):
        state = SharedState()
        state.set("n1", {"text": "hello"})
        assert state.get("n1") == {"text": "hello"}

    def test_get_nonexistent_raises(self):
        state = SharedState()
        with pytest.raises(KeyError):
            state.get("n999")

    def test_immutable_post_write(self):
        state = SharedState()
        state.set("n1", {"text": "hello"})
        with pytest.raises(ValueError, match="already set"):
            state.set("n1", {"text": "overwrite"})

    def test_get_field(self):
        state = SharedState()
        state.set("n1", {"file": "doc.pdf", "size": 1500})
        assert state.get_field("n1", "file") == "doc.pdf"
        assert state.get_field("n1", "size") == 1500

    def test_get_field_missing_node(self):
        state = SharedState()
        with pytest.raises(KeyError):
            state.get_field("n999", "field")

    def test_get_field_missing_field(self):
        state = SharedState()
        state.set("n1", {"text": "hello"})
        with pytest.raises(KeyError):
            state.get_field("n1", "nonexistent")

    def test_get_field_with_default(self):
        state = SharedState()
        state.set("n1", {"text": "hello"})
        assert state.get_field("n1", "nonexistent", default="fallback") == "fallback"

    def test_contains(self):
        state = SharedState()
        assert "n1" not in state
        state.set("n1", {"x": 1})
        assert "n1" in state

    def test_keys(self):
        state = SharedState()
        state.set("n1", {"a": 1})
        state.set("n2", {"b": 2})
        assert set(state.keys()) == {"n1", "n2"}

    def test_snapshot_returns_copy(self):
        state = SharedState()
        state.set("n1", {"a": 1})
        snap = state.snapshot()
        assert snap == {"n1": {"a": 1}}
        # Modifying snapshot doesn't affect state
        snap["n1"]["a"] = 999
        assert state.get_field("n1", "a") == 1

    def test_thread_safety(self):
        state = SharedState()
        errors = []

        def write(node_id: str):
            try:
                state.set(node_id, {"val": node_id})
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=write, args=(f"n{i}",)) for i in range(100)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert len(errors) == 0
        assert len(state.keys()) == 100

    def test_thread_safety_duplicate_write(self):
        state = SharedState()
        state.set("n1", {"val": "first"})
        errors = []

        def overwrite():
            try:
                state.set("n1", {"val": "second"})
            except ValueError:
                errors.append("caught")

        threads = [threading.Thread(target=overwrite) for _ in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert len(errors) == 10
        assert state.get("n1") == {"val": "first"}

    def test_len(self):
        state = SharedState()
        assert len(state) == 0
        state.set("n1", {"a": 1})
        assert len(state) == 1
