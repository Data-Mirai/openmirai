use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use thiserror::Error;

use super::context::{AuthContext, Role};

// ---------------------------------------------------------------------------
// Permission + Resource enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Permission {
    Read,
    Write,
    Edit,
    Execute,
    View,
    Use,
    Crud,
    Config,
    Schema,
    Deploy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Db,
    Vector,
    Storage,
    Llm,
    Agents,
}

// ---------------------------------------------------------------------------
// AuthError
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("role '{role:?}' lacks {permission:?} permission on '{resource:?}'")]
    PermissionDenied {
        role: Role,
        resource: Resource,
        permission: Permission,
    },
}

// ---------------------------------------------------------------------------
// Static permission matrix
// ---------------------------------------------------------------------------

type PermissionMatrix = HashMap<Role, HashMap<Resource, HashSet<Permission>>>;

static ROLE_PERMISSIONS: LazyLock<PermissionMatrix> = LazyLock::new(|| {
    use Permission::*;
    use Resource::*;

    let mut m = HashMap::new();

    // OWNER
    let mut owner = HashMap::new();
    owner.insert(Db, HashSet::from([Read, Write, Crud, Schema]));
    owner.insert(Vector, HashSet::from([Read, Write, Crud, Schema]));
    owner.insert(Storage, HashSet::from([Read, Write, Crud, Schema]));
    owner.insert(Llm, HashSet::from([Use, Config]));
    owner.insert(Agents, HashSet::from([View, Edit, Execute, Crud, Deploy]));
    m.insert(Role::Owner, owner);

    // ADMIN
    let mut admin = HashMap::new();
    admin.insert(Db, HashSet::from([Read, Write, Crud]));
    admin.insert(Vector, HashSet::from([Read, Write, Crud]));
    admin.insert(Storage, HashSet::from([Read, Write, Crud]));
    admin.insert(Llm, HashSet::from([Use, Config]));
    admin.insert(Agents, HashSet::from([View, Edit, Execute, Crud, Deploy]));
    m.insert(Role::Admin, admin);

    // EDITOR
    let mut editor = HashMap::new();
    editor.insert(Db, HashSet::from([Read, Write]));
    editor.insert(Vector, HashSet::from([Read, Write]));
    editor.insert(Storage, HashSet::from([Read, Write]));
    editor.insert(Llm, HashSet::from([Use]));
    editor.insert(Agents, HashSet::from([View, Edit, Execute]));
    m.insert(Role::Editor, editor);

    // VIEWER
    let mut viewer = HashMap::new();
    viewer.insert(Db, HashSet::from([Read]));
    viewer.insert(Vector, HashSet::from([Read]));
    viewer.insert(Storage, HashSet::from([Read]));
    viewer.insert(Llm, HashSet::new());
    viewer.insert(Agents, HashSet::from([View]));
    m.insert(Role::Viewer, viewer);

    m
});

// ---------------------------------------------------------------------------
// PermissionEvaluator
// ---------------------------------------------------------------------------

/// Evaluates whether a role has permission on a resource.
pub struct PermissionEvaluator;

impl PermissionEvaluator {
    /// Check if the given auth context has a specific permission on a resource.
    pub fn can(auth: &AuthContext, resource: Resource, permission: Permission) -> bool {
        ROLE_PERMISSIONS
            .get(&auth.role)
            .and_then(|res_map| res_map.get(&resource))
            .is_some_and(|perms| perms.contains(&permission))
    }

    /// Require a specific permission, returning an error if denied.
    pub fn require(
        auth: &AuthContext,
        resource: Resource,
        permission: Permission,
    ) -> Result<(), AuthError> {
        if Self::can(auth, resource, permission) {
            Ok(())
        } else {
            Err(AuthError::PermissionDenied {
                role: auth.role.clone(),
                resource,
                permission,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// SingleUserAuth
// ---------------------------------------------------------------------------

/// Single-user mode -- no restrictions. For self-hosted/dev environments.
pub struct SingleUserAuth;

impl SingleUserAuth {
    /// Create an AuthContext with Role::Owner for single-user mode.
    pub fn context(user_id: Option<&str>) -> AuthContext {
        AuthContext {
            user_id: user_id.unwrap_or("single-user").to_string(),
            role: Role::Owner,
            universe_id: None,
            environment_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_ctx() -> AuthContext {
        SingleUserAuth::context(None)
    }

    fn viewer_ctx() -> AuthContext {
        AuthContext {
            user_id: "viewer-1".into(),
            role: Role::Viewer,
            universe_id: None,
            environment_id: None,
        }
    }

    fn editor_ctx() -> AuthContext {
        AuthContext {
            user_id: "editor-1".into(),
            role: Role::Editor,
            universe_id: None,
            environment_id: None,
        }
    }

    fn admin_ctx() -> AuthContext {
        AuthContext {
            user_id: "admin-1".into(),
            role: Role::Admin,
            universe_id: None,
            environment_id: None,
        }
    }

    // --- Owner tests ---

    #[test]
    fn owner_can_do_everything() {
        let ctx = owner_ctx();
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Read
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Write
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Crud
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Schema
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Llm,
            Permission::Use
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Llm,
            Permission::Config
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Agents,
            Permission::Deploy
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Agents,
            Permission::Execute
        ));
    }

    // --- Viewer tests ---

    #[test]
    fn viewer_can_read_db() {
        let ctx = viewer_ctx();
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Read
        ));
    }

    #[test]
    fn viewer_cannot_write_db() {
        let ctx = viewer_ctx();
        assert!(!PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Write
        ));
    }

    #[test]
    fn viewer_cannot_use_llm() {
        let ctx = viewer_ctx();
        assert!(!PermissionEvaluator::can(
            &ctx,
            Resource::Llm,
            Permission::Use
        ));
    }

    #[test]
    fn viewer_can_view_agents() {
        let ctx = viewer_ctx();
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Agents,
            Permission::View
        ));
    }

    #[test]
    fn viewer_cannot_execute_agents() {
        let ctx = viewer_ctx();
        assert!(!PermissionEvaluator::can(
            &ctx,
            Resource::Agents,
            Permission::Execute
        ));
    }

    // --- Editor tests ---

    #[test]
    fn editor_can_read_write_db() {
        let ctx = editor_ctx();
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Read
        ));
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Write
        ));
    }

    #[test]
    fn editor_cannot_crud_db() {
        let ctx = editor_ctx();
        assert!(!PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Crud
        ));
    }

    #[test]
    fn editor_can_use_llm_but_not_config() {
        let ctx = editor_ctx();
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Llm,
            Permission::Use
        ));
        assert!(!PermissionEvaluator::can(
            &ctx,
            Resource::Llm,
            Permission::Config
        ));
    }

    #[test]
    fn editor_cannot_deploy_agents() {
        let ctx = editor_ctx();
        assert!(!PermissionEvaluator::can(
            &ctx,
            Resource::Agents,
            Permission::Deploy
        ));
    }

    // --- Admin tests ---

    #[test]
    fn admin_can_crud_but_not_schema() {
        let ctx = admin_ctx();
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Crud
        ));
        assert!(!PermissionEvaluator::can(
            &ctx,
            Resource::Db,
            Permission::Schema
        ));
    }

    #[test]
    fn admin_can_deploy_agents() {
        let ctx = admin_ctx();
        assert!(PermissionEvaluator::can(
            &ctx,
            Resource::Agents,
            Permission::Deploy
        ));
    }

    // --- require() tests ---

    #[test]
    fn require_ok() {
        let ctx = owner_ctx();
        assert!(PermissionEvaluator::require(&ctx, Resource::Db, Permission::Read).is_ok());
    }

    #[test]
    fn require_denied() {
        let ctx = viewer_ctx();
        let result = PermissionEvaluator::require(&ctx, Resource::Db, Permission::Write);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Viewer"));
        assert!(msg.contains("Write"));
    }

    // --- SingleUserAuth tests ---

    #[test]
    fn single_user_default_id() {
        let ctx = SingleUserAuth::context(None);
        assert_eq!(ctx.user_id, "single-user");
        assert_eq!(ctx.role, Role::Owner);
    }

    #[test]
    fn single_user_custom_id() {
        let ctx = SingleUserAuth::context(Some("dev-user"));
        assert_eq!(ctx.user_id, "dev-user");
        assert_eq!(ctx.role, Role::Owner);
    }

    // --- Serde tests ---

    #[test]
    fn permission_serde_roundtrip() {
        let p = Permission::Schema;
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"SCHEMA\"");
        let back: Permission = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Permission::Schema);
    }

    #[test]
    fn resource_serde_roundtrip() {
        let r = Resource::Llm;
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "\"llm\"");
        let back: Resource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Resource::Llm);
    }
}
