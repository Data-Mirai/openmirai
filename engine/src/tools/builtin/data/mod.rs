use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Macro: simplify boilerplate for struct + factory + spec
// ---------------------------------------------------------------------------

macro_rules! data_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        pub struct $tool;

        pub struct $factory {
            spec: ToolSpec,
        }

        impl $factory {
            pub fn new() -> Self {
                Self {
                    spec: ToolSpec {
                        tool_type: $tool_type.into(),
                        name: $name.into(),
                        description: $desc.into(),
                        version: "1.0.0".into(),
                        category: "data".into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl Default for $factory {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ToolFactory for $factory {
            fn create(&self) -> Arc<dyn Tool> {
                Arc::new($tool)
            }
            fn spec(&self) -> &ToolSpec {
                &self.spec
            }
        }
    };
}

pub mod db_read;
pub mod db_write;
pub mod entity_query;
pub mod entity_upsert;
pub mod html_to_markdown;
pub mod rag_search;
pub mod storage_read;
pub mod storage_write;
pub mod vault_read;
pub mod vault_write;
pub mod web_scrape;

// Re-export tool structs for tests and backward compat.
pub use db_read::*;
pub use db_write::*;
pub use entity_query::*;
pub use entity_upsert::*;
pub use html_to_markdown::*;
pub use rag_search::*;
pub use storage_read::*;
pub use storage_write::*;
pub use vault_read::*;
pub use vault_write::*;
pub use web_scrape::*;

pub fn register_data_tools(registry: &mut ToolRegistry) {
    registry.register("data/db_read", Box::new(DbReadFactory::new()));
    registry.register("data/db_write", Box::new(DbWriteFactory::new()));
    registry.register("data/storage_read", Box::new(StorageReadFactory::new()));
    registry.register("data/storage_write", Box::new(StorageWriteFactory::new()));
    registry.register("data/vault_read", Box::new(VaultReadFactory::new()));
    registry.register("data/vault_write", Box::new(VaultWriteFactory::new()));
    registry.register("data/entity_query", Box::new(EntityQueryFactory::new()));
    registry.register("data/entity_upsert", Box::new(EntityUpsertFactory::new()));
    registry.register("data/web_scrape", Box::new(WebScrapeFactory::new()));
    registry.register(
        "data/html_to_markdown",
        Box::new(HtmlToMarkdownFactory::new()),
    );
    registry.register("data/rag_search", Box::new(RagSearchFactory::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::{
        AuthContext, DBResource, ExecutionContext, LLMResource, LLMResponse, ResourceError, Role,
        StorageResource, TokenUsage,
    };

    // -- Stub implementations for testing ------------------------------------

    struct StubDB {
        rows: Vec<Value>,
    }

    impl StubDB {
        fn new(rows: Vec<Value>) -> Self {
            Self { rows }
        }
    }

    #[async_trait]
    impl DBResource for StubDB {
        async fn execute(&self, _query: &str, _params: &[Value]) -> Result<Value, ResourceError> {
            Ok(json!({"id": "row-1"}))
        }

        async fn fetch_one(
            &self,
            _query: &str,
            _params: &[Value],
        ) -> Result<Option<Value>, ResourceError> {
            Ok(self.rows.first().cloned())
        }

        async fn fetch_all(
            &self,
            _query: &str,
            _params: &[Value],
        ) -> Result<Vec<Value>, ResourceError> {
            Ok(self.rows.clone())
        }
    }

    struct StubStorage {
        data: HashMap<String, Vec<u8>>,
    }

    impl StubStorage {
        fn new() -> Self {
            let mut data = HashMap::new();
            data.insert("test/file.txt".to_string(), b"hello world".to_vec());
            Self { data }
        }
    }

    #[async_trait]
    impl StorageResource for StubStorage {
        async fn get(&self, path: &str) -> Result<Vec<u8>, ResourceError> {
            self.data
                .get(path)
                .cloned()
                .ok_or_else(|| ResourceError::NotFound(path.to_string()))
        }

        async fn put(&self, _path: &str, _data: &[u8]) -> Result<(), ResourceError> {
            // In a real test we'd use interior mutability; stub just succeeds.
            Ok(())
        }

        async fn delete(&self, _path: &str) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    struct StubLLM;

    #[async_trait]
    impl LLMResource for StubLLM {
        async fn call(
            &self,
            _model: &str,
            _prompt: &str,
            _context: &[Value],
            _temperature: f64,
            _max_tokens: u32,
        ) -> Result<LLMResponse, ResourceError> {
            Ok(LLMResponse {
                response: "stub".into(),
                tokens_used: TokenUsage {
                    input: 0,
                    output: 0,
                },
                model: "stub".into(),
                provider: "stub".into(),
            })
        }

        async fn embed(&self, _text: &str, _model: &str) -> Result<Vec<f64>, ResourceError> {
            Ok(vec![0.0; 128])
        }
    }

    struct TestContext {
        db: Option<Box<dyn DBResource>>,
        storage: Option<Box<dyn StorageResource>>,
        llm: StubLLM,
        auth: AuthContext,
    }

    impl TestContext {
        fn with_db(rows: Vec<Value>) -> Self {
            Self {
                db: Some(Box::new(StubDB::new(rows))),
                storage: None,
                llm: StubLLM,
                auth: AuthContext {
                    user_id: "test".into(),
                    role: Role::Owner,
                    universe_id: None,
                    environment_id: None,
                },
            }
        }

        fn with_storage() -> Self {
            Self {
                db: None,
                storage: Some(Box::new(StubStorage::new())),
                llm: StubLLM,
                auth: AuthContext {
                    user_id: "test".into(),
                    role: Role::Owner,
                    universe_id: None,
                    environment_id: None,
                },
            }
        }

        fn empty() -> Self {
            Self {
                db: None,
                storage: None,
                llm: StubLLM,
                auth: AuthContext {
                    user_id: "test".into(),
                    role: Role::Owner,
                    universe_id: None,
                    environment_id: None,
                },
            }
        }
    }

    impl ExecutionContext for TestContext {
        fn db(&self) -> Option<&dyn DBResource> {
            self.db.as_ref().map(|b| b.as_ref())
        }
        fn llm(&self) -> &dyn LLMResource {
            &self.llm
        }
        fn storage(&self) -> Option<&dyn StorageResource> {
            self.storage.as_ref().map(|b| b.as_ref())
        }
        fn vector(&self) -> Option<&dyn crate::core::context::VectorResource> {
            None
        }
        fn auth(&self) -> &AuthContext {
            &self.auth
        }
        fn session_id(&self) -> &str {
            "test-session"
        }
        fn node_id(&self) -> Option<&str> {
            None
        }
        fn system_prompt(&self) -> Option<&str> {
            None
        }
    }

    // -- DbReadTool tests -----------------------------------------------------

    #[tokio::test]
    async fn db_read_all_returns_rows() {
        let ctx = TestContext::with_db(vec![
            json!({"id": "1", "name": "Alice"}),
            json!({"id": "2", "name": "Bob"}),
        ]);

        let tool = DbReadTool;
        let inputs = HashMap::new();
        let mut config = HashMap::new();
        config.insert("query".to_string(), json!("SELECT * FROM users"));
        config.insert("mode".to_string(), json!("all"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["count"], json!(2));
        assert!(result["rows"].is_array());
    }

    #[tokio::test]
    async fn db_read_one_returns_single_row() {
        let ctx = TestContext::with_db(vec![json!({"id": "1", "name": "Alice"})]);

        let tool = DbReadTool;
        let inputs = HashMap::new();
        let mut config = HashMap::new();
        config.insert(
            "query".to_string(),
            json!("SELECT * FROM users WHERE id = 1"),
        );
        config.insert("mode".to_string(), json!("one"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["count"], json!(1));
        assert_eq!(result["row"]["name"], json!("Alice"));
    }

    #[tokio::test]
    async fn db_read_one_empty_returns_null() {
        let ctx = TestContext::with_db(vec![]);

        let tool = DbReadTool;
        let inputs = HashMap::new();
        let mut config = HashMap::new();
        config.insert(
            "query".to_string(),
            json!("SELECT * FROM users WHERE id = 999"),
        );
        config.insert("mode".to_string(), json!("one"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["count"], json!(0));
        assert!(result["row"].is_null());
    }

    #[tokio::test]
    async fn db_read_no_db_fails() {
        let ctx = TestContext::empty();
        let tool = DbReadTool;
        let result = tool.execute(HashMap::new(), &HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    // -- DbWriteTool tests ----------------------------------------------------

    #[tokio::test]
    async fn db_write_insert_returns_result() {
        let ctx = TestContext::with_db(vec![]);

        let tool = DbWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("data".to_string(), json!({"name": "Charlie"}));
        let mut config = HashMap::new();
        config.insert("table".to_string(), json!("users"));
        config.insert("mode".to_string(), json!("insert"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["table"], json!("users"));
        assert_eq!(result["action"], json!("inserted"));
        assert_eq!(result["id"], json!("row-1"));
    }

    #[tokio::test]
    async fn db_write_upsert_mode() {
        let ctx = TestContext::with_db(vec![]);

        let tool = DbWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("data".to_string(), json!({"name": "Updated"}));
        let mut config = HashMap::new();
        config.insert("table".to_string(), json!("users"));
        config.insert("mode".to_string(), json!("upsert"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["action"], json!("updated"));
    }

    #[tokio::test]
    async fn db_write_string_data_is_parsed() {
        let ctx = TestContext::with_db(vec![]);

        let tool = DbWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("data".to_string(), json!(r#"{"name": "FromString"}"#));
        let mut config = HashMap::new();
        config.insert("table".to_string(), json!("users"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["table"], json!("users"));
        assert_eq!(result["action"], json!("inserted"));
    }

    #[tokio::test]
    async fn db_write_no_db_fails() {
        let ctx = TestContext::empty();
        let tool = DbWriteTool;
        let result = tool.execute(HashMap::new(), &HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    // -- StorageReadTool tests ------------------------------------------------

    #[tokio::test]
    async fn storage_read_existing_file() {
        let ctx = TestContext::with_storage();

        let tool = StorageReadTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("test/file.txt"));
        let config = HashMap::new();

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["found"], json!(true));
        assert_eq!(result["content"], json!("hello world"));
        assert_eq!(result["path"], json!("test/file.txt"));
    }

    #[tokio::test]
    async fn storage_read_missing_file() {
        let ctx = TestContext::with_storage();

        let tool = StorageReadTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("nonexistent.txt"));
        let config = HashMap::new();

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["found"], json!(false));
        assert!(result["content"].is_null());
    }

    #[tokio::test]
    async fn storage_read_presign_mode() {
        let ctx = TestContext::with_storage();

        let tool = StorageReadTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("test/file.txt"));
        let mut config = HashMap::new();
        config.insert("mode".to_string(), json!("presign"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["found"], json!(true));
    }

    #[tokio::test]
    async fn storage_read_no_path_fails() {
        let ctx = TestContext::with_storage();
        let tool = StorageReadTool;
        let result = tool.execute(HashMap::new(), &HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn storage_read_no_storage_fails() {
        let ctx = TestContext::empty();
        let tool = StorageReadTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("test.txt"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    // -- StorageWriteTool tests -----------------------------------------------

    #[tokio::test]
    async fn storage_write_returns_bytes_written() {
        let ctx = TestContext::with_storage();

        let tool = StorageWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("output/result.txt"));
        inputs.insert("content".to_string(), json!("Hello, storage!"));
        let config = HashMap::new();

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["path"], json!("output/result.txt"));
        assert_eq!(result["bytes_written"], json!(15)); // "Hello, storage!" is 15 bytes
    }

    #[tokio::test]
    async fn storage_write_missing_path_fails() {
        let ctx = TestContext::with_storage();
        let tool = StorageWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("content".to_string(), json!("data"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn storage_write_missing_content_fails() {
        let ctx = TestContext::with_storage();
        let tool = StorageWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("output.txt"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn storage_write_no_storage_fails() {
        let ctx = TestContext::empty();
        let tool = StorageWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("out.txt"));
        inputs.insert("content".to_string(), json!("data"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    // -- VaultReadTool --------------------------------------------------------

    #[tokio::test]
    async fn vault_read_returns_empty() {
        let ctx = TestContext::empty();
        let tool = VaultReadTool;
        let result = tool
            .execute(HashMap::new(), &HashMap::new(), &ctx)
            .await
            .unwrap();
        assert_eq!(result["count"], json!(0));
        assert_eq!(result["notes"], json!([]));
    }

    // -- VaultWriteTool -------------------------------------------------------

    #[tokio::test]
    async fn vault_write_returns_path() {
        let ctx = TestContext::empty();
        let tool = VaultWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("vault/test.md"));
        inputs.insert("content".to_string(), json!("hello"));
        let result = tool.execute(inputs, &HashMap::new(), &ctx).await.unwrap();
        assert_eq!(result["path"], json!("vault/test.md"));
        assert_eq!(result["written"], json!(true));
    }

    // -- HtmlToMarkdownTool ---------------------------------------------------

    #[tokio::test]
    async fn html_to_markdown_basic() {
        let ctx = TestContext::empty();
        let tool = HtmlToMarkdownTool;
        let mut inputs = HashMap::new();
        inputs.insert(
            "html".to_string(),
            json!("<h1>Title</h1><p>Hello <strong>world</strong></p>"),
        );
        let result = tool.execute(inputs, &HashMap::new(), &ctx).await.unwrap();
        let md = result["markdown"].as_str().unwrap();
        assert!(md.contains("# Title"));
        assert!(md.contains("**world**"));
        assert!(result["length"].as_u64().unwrap() > 0);
    }

    #[test]
    fn html_to_markdown_strips_script() {
        let md = super::html_to_markdown::convert_html_to_md(
            "<p>hello</p><script>evil()</script><p>world</p>",
        );
        assert!(!md.contains("evil"));
        assert!(md.contains("hello"));
        assert!(md.contains("world"));
    }

    #[test]
    fn html_to_markdown_converts_links() {
        let md = super::html_to_markdown::convert_html_to_md(
            r#"<a href="https://example.com">Click here</a>"#,
        );
        assert!(md.contains("[Click here](https://example.com)"));
    }

    // -- Registration ---------------------------------------------------------

    #[test]
    fn register_data_tools_adds_ten() {
        let mut reg = ToolRegistry::new();
        register_data_tools(&mut reg);
        assert!(reg.get("data/db_read").is_some());
        assert!(reg.get("data/db_write").is_some());
        assert!(reg.get("data/storage_read").is_some());
        assert!(reg.get("data/storage_write").is_some());
        assert!(reg.get("data/vault_read").is_some());
        assert!(reg.get("data/vault_write").is_some());
        assert!(reg.get("data/entity_query").is_some());
        assert!(reg.get("data/entity_upsert").is_some());
        assert!(reg.get("data/web_scrape").is_some());
        assert!(reg.get("data/html_to_markdown").is_some());
        assert_eq!(reg.list_tools().len(), 11);
    }

    // =========================================================================
    // WebScrapeTool: SessionFingerprint tests
    // =========================================================================

    #[test]
    fn fingerprint_deterministic_for_same_session() {
        let fp1 = super::SessionFingerprint::generate("session-abc");
        let fp2 = super::SessionFingerprint::generate("session-abc");
        assert_eq!(fp1.user_agent, fp2.user_agent);
        assert_eq!(fp1.viewport, fp2.viewport);
        assert_eq!(fp1.platform, fp2.platform);
        assert_eq!(fp1.browser, fp2.browser);
        assert_eq!(fp1.locale, fp2.locale);
    }

    #[test]
    fn fingerprint_different_for_different_sessions() {
        let fp1 = super::SessionFingerprint::generate("session-abc");
        let fp2 = super::SessionFingerprint::generate("session-xyz");
        let same = fp1.user_agent == fp2.user_agent
            && fp1.viewport == fp2.viewport
            && fp1.locale == fp2.locale;
        assert!(
            !same,
            "Different session IDs should produce different fingerprints"
        );
    }

    #[test]
    fn stealth_headers_contain_required_fields() {
        let fp = super::SessionFingerprint::generate("stealth-test");
        let headers = fp.build_headers();
        assert!(headers.get("user-agent").is_some());
        assert!(headers.get("accept").is_some());
        assert!(headers.get("accept-language").is_some());
        assert!(headers.get("accept-encoding").is_some());
        assert!(headers.get("dnt").is_some());
        assert!(headers.get("upgrade-insecure-requests").is_some());
    }

    // =========================================================================
    // WebScrapeTool: URL normalization
    // =========================================================================

    #[test]
    fn url_normalization_strips_tracking_params() {
        let raw = "https://Example.COM/page?utm_source=google&foo=bar&fbclid=abc123&q=test";
        let normalized = super::normalize_url(raw);
        assert!(!normalized.contains("utm_source"));
        assert!(!normalized.contains("fbclid"));
        assert!(normalized.contains("foo=bar"));
        assert!(normalized.contains("q=test"));
        assert!(normalized.contains("example.com"));
    }

    // =========================================================================
    // WebScrapeTool: SERP extraction
    // =========================================================================

    #[test]
    fn serp_extraction_google() {
        let html = r#"
            <div>
                <a href="/url?q=https://example.com/article1&sa=U">Result 1</a>
                <a href="/url?q=https://example.org/news&sa=U">Result 2</a>
                <a href="/url?q=https://google.com/maps&sa=U">Maps</a>
            </div>
        "#;
        let links = super::extract_serp_links(html, "google");
        assert!(links.len() >= 2);
        assert!(links.iter().any(|l| l.contains("example.com/article1")));
        assert!(links.iter().any(|l| l.contains("example.org/news")));
        assert!(!links.iter().any(|l| l.contains("google.com/maps")));
    }

    #[test]
    fn serp_extraction_duckduckgo() {
        let html = r#"
            <a class="result__a" href="https://example.com/ddg-result">DDG Result</a>
            <a href="?uddg=https%3A%2F%2Fexample.net%2Farticle">Another</a>
        "#;
        let links = super::extract_serp_links(html, "duckduckgo");
        assert!(!links.is_empty());
    }

    // =========================================================================
    // WebScrapeTool: jitter
    // =========================================================================

    #[test]
    fn jitter_stays_within_30_percent_range() {
        let base = std::time::Duration::from_millis(1000);
        for _ in 0..100 {
            let jittered = super::apply_jitter(base);
            let ms = jittered.as_millis();
            assert!(
                (700..=1300).contains(&ms),
                "Jitter {ms}ms is outside [700, 1300] range"
            );
        }
    }

    // =========================================================================
    // WebScrapeTool: cache
    // =========================================================================

    #[tokio::test]
    async fn cache_returns_cached_response_within_ttl() {
        let state = super::ScrapeState::new();
        let url = "https://example.com/page";
        let normalized = super::normalize_url(url);

        {
            let mut cache = state.cache.lock().await;
            cache.insert(
                normalized.clone(),
                super::CachedResponse {
                    body: "cached body".into(),
                    status: 200,
                    fetched_at: std::time::Instant::now(),
                    ttl: std::time::Duration::from_secs(300),
                },
            );
        }

        let cache = state.cache.lock().await;
        let cached = cache.get(&normalized);
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert!(cached.is_valid());
        assert_eq!(cached.body, "cached body");
        assert_eq!(cached.status, 200);
    }

    #[tokio::test]
    async fn cache_misses_after_ttl_expires() {
        let state = super::ScrapeState::new();
        let url = "https://example.com/expired";
        let normalized = super::normalize_url(url);

        {
            let mut cache = state.cache.lock().await;
            cache.insert(
                normalized.clone(),
                super::CachedResponse {
                    body: "old body".into(),
                    status: 200,
                    fetched_at: std::time::Instant::now() - std::time::Duration::from_secs(10),
                    ttl: std::time::Duration::from_secs(1),
                },
            );
        }

        let cache = state.cache.lock().await;
        let cached = cache.get(&normalized);
        assert!(cached.is_some());
        assert!(!cached.unwrap().is_valid());
    }
}
