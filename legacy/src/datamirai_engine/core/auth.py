"""Auth — Role-based permission evaluation.

Roles: OWNER > ADMIN > EDITOR > VIEWER.
Engine evaluates permissions on resources/agents.
Engine does NOT handle login/signup/tokens — receives verified identity.
"""

from __future__ import annotations

from enum import StrEnum

from datamirai_engine.core.context import AuthContext


class Permission(StrEnum):
    READ = "READ"
    WRITE = "WRITE"
    EDIT = "EDIT"
    EXECUTE = "EXECUTE"
    VIEW = "VIEW"
    USE = "USE"
    CRUD = "CRUD"
    CONFIG = "CONFIG"
    SCHEMA = "SCHEMA"
    DEPLOY = "DEPLOY"


# Permission matrix: role → resource → set of allowed permissions
_ROLE_PERMISSIONS: dict[str, dict[str, set[Permission]]] = {
    "OWNER": {
        "db": {Permission.READ, Permission.WRITE, Permission.CRUD, Permission.SCHEMA},
        "vector": {Permission.READ, Permission.WRITE, Permission.CRUD, Permission.SCHEMA},
        "storage": {Permission.READ, Permission.WRITE, Permission.CRUD, Permission.SCHEMA},
        "llm": {Permission.USE, Permission.CONFIG},
        "agents": {
            Permission.VIEW, Permission.EDIT, Permission.EXECUTE,
            Permission.CRUD, Permission.DEPLOY,
        },
    },
    "ADMIN": {
        "db": {Permission.READ, Permission.WRITE, Permission.CRUD},
        "vector": {Permission.READ, Permission.WRITE, Permission.CRUD},
        "storage": {Permission.READ, Permission.WRITE, Permission.CRUD},
        "llm": {Permission.USE, Permission.CONFIG},
        "agents": {
            Permission.VIEW, Permission.EDIT, Permission.EXECUTE,
            Permission.CRUD, Permission.DEPLOY,
        },
    },
    "EDITOR": {
        "db": {Permission.READ, Permission.WRITE},
        "vector": {Permission.READ, Permission.WRITE},
        "storage": {Permission.READ, Permission.WRITE},
        "llm": {Permission.USE},
        "agents": {Permission.VIEW, Permission.EDIT, Permission.EXECUTE},
    },
    "VIEWER": {
        "db": {Permission.READ},
        "vector": {Permission.READ},
        "storage": {Permission.READ},
        "llm": set(),
        "agents": {Permission.VIEW},
    },
}


class PermissionEvaluator:
    """Evaluates whether a role has permission on a resource."""

    def can(self, auth: AuthContext, resource: str, permission: Permission) -> bool:
        role_perms = _ROLE_PERMISSIONS.get(auth.role, {})
        resource_perms = role_perms.get(resource, set())
        return permission in resource_perms

    def require(
        self, auth: AuthContext, resource: str, permission: Permission
    ) -> None:
        if not self.can(auth, resource, permission):
            raise PermissionError(
                f"Role '{auth.role}' lacks {permission.value} permission on '{resource}'"
            )


class _AlwaysAllowEvaluator(PermissionEvaluator):
    def can(self, auth: AuthContext, resource: str, permission: Permission) -> bool:
        return True


class SingleUserAuth:
    """Single-user mode — no restrictions. For self-hosted/dev."""

    @staticmethod
    def context(user_id: str = "single-user") -> AuthContext:
        return AuthContext(user_id=user_id, role="OWNER")

    @staticmethod
    def evaluator() -> PermissionEvaluator:
        return _AlwaysAllowEvaluator()
