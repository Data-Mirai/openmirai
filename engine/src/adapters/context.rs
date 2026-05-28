//! DefaultExecutionContext — wires all resources together into a concrete
//! `ExecutionContext` implementation, with a builder pattern for ergonomic
//! construction.

use crate::core::context::{
    AuthContext, DBResource, ExecutionContext, LLMResource, Role, StorageResource, VectorResource,
};

// ---------------------------------------------------------------------------
// DefaultExecutionContext
// ---------------------------------------------------------------------------

/// Concrete implementation of `ExecutionContext` that holds boxed resource
/// trait objects.  Suitable for both production and testing.
pub struct DefaultExecutionContext {
    db: Option<Box<dyn DBResource>>,
    llm: Box<dyn LLMResource>,
    storage: Option<Box<dyn StorageResource>>,
    vector: Option<Box<dyn VectorResource>>,
    auth: AuthContext,
    session_id: String,
    node_id: Option<String>,
    system_prompt: Option<String>,
    scratch_dir: Option<String>,
}

impl ExecutionContext for DefaultExecutionContext {
    fn db(&self) -> Option<&dyn DBResource> {
        self.db.as_deref()
    }

    fn llm(&self) -> &dyn LLMResource {
        &*self.llm
    }

    fn storage(&self) -> Option<&dyn StorageResource> {
        self.storage.as_deref()
    }

    fn vector(&self) -> Option<&dyn VectorResource> {
        self.vector.as_deref()
    }

    fn auth(&self) -> &AuthContext {
        &self.auth
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn node_id(&self) -> Option<&str> {
        self.node_id.as_deref()
    }

    fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    fn scratch_dir(&self) -> Option<&str> {
        self.scratch_dir.as_deref()
    }
}

impl DefaultExecutionContext {
    /// Start building a context.  The LLM resource is always required.
    pub fn builder(llm: Box<dyn LLMResource>) -> DefaultExecutionContextBuilder {
        DefaultExecutionContextBuilder {
            db: None,
            llm,
            storage: None,
            vector: None,
            auth: AuthContext {
                user_id: "dev".to_string(),
                role: Role::Owner,
                universe_id: None,
                environment_id: None,
            },
            session_id: crate::utils::short_id(),
            node_id: None,
            system_prompt: None,
            scratch_dir: None,
        }
    }

    /// Convenience constructor for a fully in-memory dev/testing context.
    pub fn default_dev() -> Self {
        use super::in_memory_db::InMemoryDBResource;
        use super::in_memory_storage::InMemoryStorageResource;
        use super::mock_llm::MockLLMResource;

        Self::builder(Box::new(MockLLMResource::new()))
            .with_db(Box::new(InMemoryDBResource::new()))
            .with_storage(Box::new(InMemoryStorageResource::new()))
            .build()
    }

    /// Set the node_id (used by the runner during execution).
    pub fn set_node_id(&mut self, node_id: Option<String>) {
        self.node_id = node_id;
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for `DefaultExecutionContext`.
pub struct DefaultExecutionContextBuilder {
    db: Option<Box<dyn DBResource>>,
    llm: Box<dyn LLMResource>,
    storage: Option<Box<dyn StorageResource>>,
    vector: Option<Box<dyn VectorResource>>,
    auth: AuthContext,
    session_id: String,
    node_id: Option<String>,
    system_prompt: Option<String>,
    scratch_dir: Option<String>,
}

impl DefaultExecutionContextBuilder {
    pub fn with_db(mut self, db: Box<dyn DBResource>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn with_storage(mut self, storage: Box<dyn StorageResource>) -> Self {
        self.storage = Some(storage);
        self
    }

    pub fn with_vector(mut self, vector: Box<dyn VectorResource>) -> Self {
        self.vector = Some(vector);
        self
    }

    pub fn with_auth(mut self, auth: AuthContext) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }

    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_scratch_dir(mut self, path: impl Into<String>) -> Self {
        self.scratch_dir = Some(path.into());
        self
    }

    pub fn build(self) -> DefaultExecutionContext {
        DefaultExecutionContext {
            db: self.db,
            llm: self.llm,
            storage: self.storage,
            vector: self.vector,
            auth: self.auth,
            session_id: self.session_id,
            node_id: self.node_id,
            system_prompt: self.system_prompt,
            scratch_dir: self.scratch_dir,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::Role;

    #[test]
    fn default_dev_context() {
        let ctx = DefaultExecutionContext::default_dev();
        assert!(ctx.db().is_some());
        assert!(ctx.storage().is_some());
        assert_eq!(ctx.auth().role, Role::Owner);
        assert_eq!(ctx.auth().user_id, "dev");
        assert!(!ctx.session_id().is_empty());
        assert!(ctx.node_id().is_none());
        assert!(ctx.system_prompt().is_none());
    }

    #[test]
    fn builder_with_all_options() {
        use super::super::mock_llm::MockLLMResource;

        let ctx = DefaultExecutionContext::builder(Box::new(MockLLMResource::new()))
            .with_auth(AuthContext {
                user_id: "u-1".into(),
                role: Role::Editor,
                universe_id: Some("uni".into()),
                environment_id: None,
            })
            .with_session_id("sess-42")
            .with_node_id("n-1")
            .with_system_prompt("You are helpful.")
            .build();

        assert_eq!(ctx.auth().user_id, "u-1");
        assert_eq!(ctx.auth().role, Role::Editor);
        assert_eq!(ctx.session_id(), "sess-42");
        assert_eq!(ctx.node_id(), Some("n-1"));
        assert_eq!(ctx.system_prompt(), Some("You are helpful."));
        assert!(ctx.db().is_none());
        assert!(ctx.storage().is_none());
    }

    #[test]
    fn set_node_id_mutates() {
        let mut ctx = DefaultExecutionContext::default_dev();
        assert!(ctx.node_id().is_none());
        ctx.set_node_id(Some("n-5".to_string()));
        assert_eq!(ctx.node_id(), Some("n-5"));
        ctx.set_node_id(None);
        assert!(ctx.node_id().is_none());
    }
}
