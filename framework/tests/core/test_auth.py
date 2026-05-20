"""Tests for Auth — roles, permissions, single-user mode."""

from __future__ import annotations

import pytest

from datamirai_engine.core.auth import Permission, PermissionEvaluator, SingleUserAuth
from datamirai_engine.core.context import AuthContext


class TestPermissionEvaluator:
    @pytest.fixture
    def evaluator(self):
        return PermissionEvaluator()

    def test_owner_can_do_everything(self, evaluator):
        auth = AuthContext(user_id="u1", role="OWNER")
        assert evaluator.can(auth, "agents", Permission.CRUD)
        assert evaluator.can(auth, "db", Permission.SCHEMA)
        assert evaluator.can(auth, "llm", Permission.CONFIG)

    def test_admin_can_crud(self, evaluator):
        auth = AuthContext(user_id="u1", role="ADMIN")
        assert evaluator.can(auth, "agents", Permission.CRUD)
        assert evaluator.can(auth, "db", Permission.CRUD)
        assert evaluator.can(auth, "llm", Permission.CONFIG)

    def test_admin_cannot_schema(self, evaluator):
        auth = AuthContext(user_id="u1", role="ADMIN")
        assert not evaluator.can(auth, "db", Permission.SCHEMA)

    def test_editor_can_read_write(self, evaluator):
        auth = AuthContext(user_id="u1", role="EDITOR")
        assert evaluator.can(auth, "db", Permission.READ)
        assert evaluator.can(auth, "db", Permission.WRITE)
        assert evaluator.can(auth, "agents", Permission.EDIT)
        assert evaluator.can(auth, "agents", Permission.EXECUTE)

    def test_editor_cannot_crud_agents(self, evaluator):
        auth = AuthContext(user_id="u1", role="EDITOR")
        assert not evaluator.can(auth, "agents", Permission.CRUD)

    def test_viewer_read_only(self, evaluator):
        auth = AuthContext(user_id="u1", role="VIEWER")
        assert evaluator.can(auth, "db", Permission.READ)
        assert evaluator.can(auth, "agents", Permission.VIEW)
        assert not evaluator.can(auth, "db", Permission.WRITE)
        assert not evaluator.can(auth, "agents", Permission.EDIT)
        assert not evaluator.can(auth, "llm", Permission.USE)

    def test_unknown_role_denied(self, evaluator):
        auth = AuthContext(user_id="u1", role="HACKER")
        assert not evaluator.can(auth, "db", Permission.READ)

    def test_require_raises_on_denied(self, evaluator):
        auth = AuthContext(user_id="u1", role="VIEWER")
        with pytest.raises(PermissionError, match=r"VIEWER.*WRITE.*db"):
            evaluator.require(auth, "db", Permission.WRITE)

    def test_require_passes_on_allowed(self, evaluator):
        auth = AuthContext(user_id="u1", role="OWNER")
        evaluator.require(auth, "db", Permission.SCHEMA)  # should not raise


class TestSingleUserAuth:
    def test_creates_owner_context(self):
        auth = SingleUserAuth.context()
        assert isinstance(auth, AuthContext)
        assert auth.role == "OWNER"
        assert auth.user_id == "single-user"

    def test_evaluator_always_allows(self):
        evaluator = SingleUserAuth.evaluator()
        auth = SingleUserAuth.context()
        assert evaluator.can(auth, "anything", Permission.SCHEMA)

    def test_custom_user_id(self):
        auth = SingleUserAuth.context(user_id="dev-user")
        assert auth.user_id == "dev-user"
