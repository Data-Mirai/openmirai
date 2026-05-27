use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{FieldType, ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder (same pattern as logic.rs)
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type,
        required,
        description: if desc.is_empty() {
            None
        } else {
            Some(desc.into())
        },
        default: None,
    }
}

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

// ===========================================================================
// DbReadTool
// ===========================================================================

data_tool! {
    struct DbReadTool, factory DbReadFactory;
    tool_type = "data/db_read",
    name = "DB Read",
    description = "Reads data from relational database with filtering and pagination",
    inputs = [
        field("query_params", FieldType::Object, false, "Filter parameters: {column: value} for WHERE clause"),
    ],
    outputs = [
        field("rows", FieldType::Array, false, "Array of matched rows (mode=all)"),
        field("row", FieldType::Object, false, "Single matched row (mode=one)"),
        field("count", FieldType::Number, true, "Number of rows returned"),
    ],
    config_fields = [
        field("query", FieldType::String, true, "SQL query to execute"),
        field("mode", FieldType::String, false, "Read mode: 'one' or 'all' (default: all)"),
    ]
}

#[async_trait]
impl Tool for DbReadTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/db_read".into(),
            message: "no database resource configured".into(),
        })?;

        let query = config
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("SELECT");

        let mode = config
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        // Build params from query_params input
        let params: Vec<Value> = match inputs.get("query_params") {
            Some(Value::Object(map)) => map.values().cloned().collect(),
            _ => vec![],
        };

        let mut out = HashMap::new();

        if mode == "one" {
            let row = db
                .fetch_one(query, &params)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "data/db_read".into(),
                    message: e.to_string(),
                })?;

            let count = if row.is_some() { 1 } else { 0 };
            out.insert("row".to_string(), row.unwrap_or(Value::Null));
            out.insert("count".to_string(), json!(count));
        } else {
            let rows = db
                .fetch_all(query, &params)
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool_type: "data/db_read".into(),
                    message: e.to_string(),
                })?;

            let count = rows.len();
            out.insert("rows".to_string(), json!(rows));
            out.insert("count".to_string(), json!(count));
        }

        Ok(out)
    }
}

// ===========================================================================
// DbWriteTool
// ===========================================================================

data_tool! {
    struct DbWriteTool, factory DbWriteFactory;
    tool_type = "data/db_write",
    name = "DB Write",
    description = "Writes data to relational database with automatic table creation",
    inputs = [
        field("data", FieldType::Object, true, "Row data to write"),
    ],
    outputs = [
        field("table", FieldType::String, true, "Target table name"),
        field("action", FieldType::String, true, "Action performed: inserted or updated"),
        field("id", FieldType::String, false, "Row ID of the written record"),
    ],
    config_fields = [
        field("table", FieldType::String, true, "Target table name"),
        field("mode", FieldType::String, false, "Write mode: 'insert' or 'upsert' (default: insert)"),
    ]
}

#[async_trait]
impl Tool for DbWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/db_write".into(),
            message: "no database resource configured".into(),
        })?;

        let table = config
            .get("table")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let mode = config
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("insert");

        // Get data from input, normalizing from string if needed
        let data = match inputs.get("data") {
            Some(Value::String(s)) => {
                // Try to parse JSON string
                serde_json::from_str::<Value>(s)
                    .unwrap_or_else(|_| json!({"content": s}))
            }
            Some(Value::Object(_)) => inputs.get("data").cloned().unwrap(),
            Some(other) => json!({"content": other.to_string()}),
            None => json!({}),
        };

        // Build SQL based on mode
        let action = if mode == "upsert" { "updated" } else { "inserted" };

        // Use the data as params for the execute call
        let params = vec![json!(table), data.clone(), json!(mode)];

        let result = db
            .execute("INSERT", &params)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/db_write".into(),
                message: e.to_string(),
            })?;

        // Extract row_id from result
        let row_id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut out = HashMap::new();
        out.insert("table".to_string(), json!(table));
        out.insert("action".to_string(), json!(action));
        out.insert("id".to_string(), json!(row_id));
        Ok(out)
    }
}

// ===========================================================================
// StorageReadTool
// ===========================================================================

data_tool! {
    struct StorageReadTool, factory StorageReadFactory;
    tool_type = "data/storage_read",
    name = "Storage Read",
    description = "Reads file from S3-compatible storage or generates presigned URL",
    inputs = [
        field("path", FieldType::String, true, "Storage path/key to read"),
    ],
    outputs = [
        field("content", FieldType::String, false, "File content as text"),
        field("path", FieldType::String, true, "Path that was read"),
        field("found", FieldType::Boolean, true, "Whether the file was found"),
    ],
    config_fields = [
        field("mode", FieldType::String, false, "Read mode: 'read' or 'presign' (default: read)"),
    ]
}

#[async_trait]
impl Tool for StorageReadTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let storage = context.storage().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/storage_read".into(),
            message: "no storage resource configured".into(),
        })?;

        let path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/storage_read".into(),
                message: "input 'path' is required".into(),
            })?;

        let mode = config
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("read");

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path));

        if mode == "presign" {
            // Presigned URL mode: for now, construct a placeholder path-based URL.
            // A real implementation would delegate to the storage resource's presign method.
            out.insert("content".to_string(), Value::Null);
            out.insert("found".to_string(), json!(true));
            return Ok(out);
        }

        // Normal read mode
        match storage.get(path).await {
            Ok(data) => {
                let content = String::from_utf8(data)
                    .unwrap_or_else(|e| format!("<binary data: {} bytes>", e.as_bytes().len()));
                out.insert("content".to_string(), json!(content));
                out.insert("found".to_string(), json!(true));
            }
            Err(crate::core::context::ResourceError::NotFound(_)) => {
                out.insert("content".to_string(), Value::Null);
                out.insert("found".to_string(), json!(false));
            }
            Err(e) => {
                return Err(ToolError::ExecutionFailed {
                    tool_type: "data/storage_read".into(),
                    message: e.to_string(),
                });
            }
        }

        Ok(out)
    }
}

// ===========================================================================
// StorageWriteTool
// ===========================================================================

data_tool! {
    struct StorageWriteTool, factory StorageWriteFactory;
    tool_type = "data/storage_write",
    name = "Storage Write",
    description = "Writes file to S3-compatible storage",
    inputs = [
        field("path", FieldType::String, true, "Storage path/key to write to"),
        field("content", FieldType::String, true, "Content to write"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Path that was written"),
        field("bytes_written", FieldType::Number, true, "Number of bytes written"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for StorageWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let storage = context.storage().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/storage_write".into(),
            message: "no storage resource configured".into(),
        })?;

        let path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/storage_write".into(),
                message: "input 'path' is required".into(),
            })?;

        let content = inputs
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/storage_write".into(),
                message: "input 'content' is required".into(),
            })?;

        let data = content.as_bytes();
        let bytes_written = data.len();

        storage
            .put(path, data)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/storage_write".into(),
                message: e.to_string(),
            })?;

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path));
        out.insert("bytes_written".to_string(), json!(bytes_written));
        Ok(out)
    }
}

// ===========================================================================
// VaultReadTool
// ===========================================================================

data_tool! {
    struct VaultReadTool, factory VaultReadFactory;
    tool_type = "data/vault_read",
    name = "Vault Read",
    description = "Reads notes from the Knowledge Vault. Placeholder that returns empty results.",
    inputs = [
        field("query", FieldType::String, false, "Search query for vault notes"),
        field("path", FieldType::String, false, "Specific vault path to read"),
    ],
    outputs = [
        field("notes", FieldType::Array, true, "Matched vault notes"),
        field("count", FieldType::Number, true, "Number of notes returned"),
    ],
    config_fields = [
        field("folder", FieldType::String, false, "Vault folder to search in"),
        field("limit", FieldType::Number, false, "Maximum notes to return"),
    ]
}

#[async_trait]
impl Tool for VaultReadTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        // Placeholder: vault requires a filesystem backend.
        // In a real implementation this would query the vault service.
        let mut out = HashMap::new();
        out.insert("notes".to_string(), json!([]));
        out.insert("count".to_string(), json!(0));
        Ok(out)
    }
}

// ===========================================================================
// VaultWriteTool
// ===========================================================================

data_tool! {
    struct VaultWriteTool, factory VaultWriteFactory;
    tool_type = "data/vault_write",
    name = "Vault Write",
    description = "Writes a note to the Knowledge Vault. Placeholder that acknowledges the write.",
    inputs = [
        field("path", FieldType::String, true, "Vault path for the note"),
        field("title", FieldType::String, false, "Note title"),
        field("content", FieldType::String, true, "Note content"),
    ],
    outputs = [
        field("path", FieldType::String, true, "Path where note was written"),
        field("written", FieldType::Boolean, true, "Whether write succeeded"),
    ],
    config_fields = [
        field("tags", FieldType::String, false, "Comma-separated tags"),
    ]
}

#[async_trait]
impl Tool for VaultWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let path = inputs
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("vault/untitled.md");

        let mut out = HashMap::new();
        out.insert("path".to_string(), json!(path));
        out.insert("written".to_string(), json!(true));
        Ok(out)
    }
}

// ===========================================================================
// EntityQueryTool
// ===========================================================================

data_tool! {
    struct EntityQueryTool, factory EntityQueryFactory;
    tool_type = "data/entity_query",
    name = "Entity Query",
    description = "Queries entities from the database with field extraction",
    inputs = [
        field("entity_type", FieldType::String, true, "Entity type to query"),
        field("filters", FieldType::Object, false, "Filter conditions as {field: value}"),
    ],
    outputs = [
        field("entities", FieldType::Array, true, "Matched entities"),
        field("count", FieldType::Number, true, "Number of entities returned"),
    ],
    config_fields = [
        field("limit", FieldType::Number, false, "Maximum entities to return"),
        field("order_by", FieldType::String, false, "Field to order by"),
    ]
}

#[async_trait]
impl Tool for EntityQueryTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/entity_query".into(),
            message: "no database resource configured".into(),
        })?;

        let entity_type = inputs
            .get("entity_type")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let limit = config
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);

        let query = format!("SELECT * FROM entities WHERE type = ? LIMIT {}", limit);
        let params = vec![json!(entity_type)];

        let rows = db
            .fetch_all(&query, &params)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/entity_query".into(),
                message: e.to_string(),
            })?;

        let count = rows.len();
        let mut out = HashMap::new();
        out.insert("entities".to_string(), json!(rows));
        out.insert("count".to_string(), json!(count));
        Ok(out)
    }
}

// ===========================================================================
// EntityUpsertTool
// ===========================================================================

data_tool! {
    struct EntityUpsertTool, factory EntityUpsertFactory;
    tool_type = "data/entity_upsert",
    name = "Entity Upsert",
    description = "Creates or updates an entity in the database",
    inputs = [
        field("entity_type", FieldType::String, true, "Entity type"),
        field("data", FieldType::Object, true, "Entity field data"),
        field("id", FieldType::String, false, "Entity ID (if updating)"),
    ],
    outputs = [
        field("id", FieldType::String, true, "Entity ID"),
        field("action", FieldType::String, true, "Action performed: created or updated"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for EntityUpsertTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let db = context.db().ok_or_else(|| ToolError::ExecutionFailed {
            tool_type: "data/entity_upsert".into(),
            message: "no database resource configured".into(),
        })?;

        let entity_type = inputs
            .get("entity_type")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let data = inputs
            .get("data")
            .cloned()
            .unwrap_or(json!({}));
        let existing_id = inputs
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        let action = if existing_id.is_some() { "updated" } else { "created" };

        let id = existing_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let params = vec![json!(id), json!(entity_type), data];
        db.execute("UPSERT", &params)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/entity_upsert".into(),
                message: e.to_string(),
            })?;

        let mut out = HashMap::new();
        out.insert("id".to_string(), json!(id));
        out.insert("action".to_string(), json!(action));
        Ok(out)
    }
}

// ===========================================================================
// WebScrapeTool -- production-grade stealth scraper
// ===========================================================================

// ---------------------------------------------------------------------------
// Stealth fingerprinting
// ---------------------------------------------------------------------------

/// 12 modern user agents from real browsers (2024-2026).
const USER_AGENT_POOL: &[&str] = &[
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:126.0) Gecko/20100101 Firefox/126.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:126.0) Gecko/20100101 Firefox/126.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:126.0) Gecko/20100101 Firefox/126.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36 Edg/125.0.0.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36 OPR/111.0.0.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36",
];

/// 5 common desktop viewport sizes.
const VIEWPORT_POOL: &[(u32, u32)] = &[
    (1920, 1080),
    (1366, 768),
    (1280, 800),
    (1440, 900),
    (1536, 864),
];

/// Deterministic session fingerprint derived from a session_id hash.
#[derive(Debug, Clone)]
pub struct SessionFingerprint {
    pub user_agent: String,
    pub viewport: (u32, u32),
    pub platform: String,
    pub browser: String,
    pub locale: String,
}

impl SessionFingerprint {
    /// Generate a deterministic fingerprint from the given session_id.
    /// The same session_id always produces the same fingerprint.
    pub fn generate(session_id: &str) -> Self {
        use sha2::{Digest, Sha256};

        let hash = Sha256::digest(session_id.as_bytes());
        let seed = u64::from_le_bytes(hash[..8].try_into().unwrap());

        let ua_idx = (seed as usize) % USER_AGENT_POOL.len();
        let vp_idx = ((seed >> 16) as usize) % VIEWPORT_POOL.len();

        let ua = USER_AGENT_POOL[ua_idx];
        let viewport = VIEWPORT_POOL[vp_idx];

        let browser = detect_browser(ua);
        let platform = detect_platform(ua);

        let locales = ["es-ES", "en-US", "es-MX", "en-GB"];
        let locale = locales[((seed >> 32) as usize) % locales.len()];

        Self {
            user_agent: ua.to_string(),
            viewport,
            platform: platform.to_string(),
            browser: browser.to_string(),
            locale: locale.to_string(),
        }
    }

    /// Build HTTP headers from this fingerprint.
    pub fn build_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

        let mut headers = HeaderMap::new();

        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str(&self.user_agent).unwrap_or_else(|_| {
                HeaderValue::from_static("Mozilla/5.0")
            }),
        );

        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            ),
        );

        let accept_lang = format!(
            "{},{};q=0.9,en-US;q=0.8,en;q=0.7",
            self.locale,
            self.locale.split('-').next().unwrap_or("en"),
        );
        if let Ok(val) = HeaderValue::from_str(&accept_lang) {
            headers.insert(reqwest::header::ACCEPT_LANGUAGE, val);
        }

        headers.insert(
            reqwest::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br"),
        );

        headers.insert(
            HeaderName::from_static("dnt"),
            HeaderValue::from_static("1"),
        );

        headers.insert(
            HeaderName::from_static("upgrade-insecure-requests"),
            HeaderValue::from_static("1"),
        );

        // Sec-CH-UA headers only for Chromium-based browsers
        if matches!(self.browser.as_str(), "chrome" | "edge" | "opera") {
            let version = extract_chrome_version(&self.user_agent);

            let sec_ch_ua = match self.browser.as_str() {
                "chrome" => format!(
                    "\"Chromium\";v=\"{version}\", \"Google Chrome\";v=\"{version}\", \"Not.A/Brand\";v=\"24\""
                ),
                "edge" => format!(
                    "\"Chromium\";v=\"{version}\", \"Microsoft Edge\";v=\"{version}\", \"Not.A/Brand\";v=\"24\""
                ),
                "opera" => format!(
                    "\"Chromium\";v=\"{version}\", \"Opera\";v=\"111\", \"Not.A/Brand\";v=\"24\""
                ),
                _ => String::new(),
            };

            if let Ok(val) = HeaderValue::from_str(&sec_ch_ua) {
                headers.insert(HeaderName::from_static("sec-ch-ua"), val);
            }
            headers.insert(
                HeaderName::from_static("sec-ch-ua-mobile"),
                HeaderValue::from_static("?0"),
            );
            let platform_quoted = format!("\"{}\"", self.platform);
            if let Ok(val) = HeaderValue::from_str(&platform_quoted) {
                headers.insert(HeaderName::from_static("sec-ch-ua-platform"), val);
            }
        }

        headers
    }
}

fn detect_browser(ua: &str) -> &'static str {
    if ua.contains("Edg/") {
        "edge"
    } else if ua.contains("OPR/") {
        "opera"
    } else if ua.contains("Firefox/") {
        "firefox"
    } else if ua.contains("Safari/") && !ua.contains("Chrome/") {
        "safari"
    } else if ua.contains("Chrome/") {
        "chrome"
    } else {
        "unknown"
    }
}

fn detect_platform(ua: &str) -> &'static str {
    if ua.contains("Macintosh") {
        "macOS"
    } else if ua.contains("Windows NT") {
        "Windows"
    } else if ua.contains("X11; Linux") || ua.contains("Linux") {
        "Linux"
    } else {
        "Windows"
    }
}

fn extract_chrome_version(ua: &str) -> String {
    let re = regex::Regex::new(r"Chrome/(\d+)").unwrap();
    re.captures(ua)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "125".into())
}

// ---------------------------------------------------------------------------
// Jitter
// ---------------------------------------------------------------------------

/// Apply +/-30% random jitter to a delay value.
fn apply_jitter(delay: Duration) -> Duration {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let factor: f64 = 0.7 + rng.gen::<f64>() * 0.6; // [0.7, 1.3]
    delay.mul_f64(factor)
}

// ---------------------------------------------------------------------------
// URL normalization / cache
// ---------------------------------------------------------------------------

/// Tracking params to strip when normalizing URLs for cache keys.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "ref",
    "mc_cid",
    "mc_eid",
];

/// Normalize a URL: lowercase host, sort query params, strip tracking params.
fn normalize_url(raw: &str) -> String {
    let parsed = match url::Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.to_string(),
    };

    let filtered: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !TRACKING_PARAMS.contains(&k.to_lowercase().as_str()))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let mut sorted = filtered;
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let base = format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or("").to_lowercase(),
        parsed.path()
    );

    if sorted.is_empty() {
        base
    } else {
        let qs: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{base}?{}", qs.join("&"))
    }
}

/// Cached HTTP response.
#[derive(Debug, Clone)]
struct CachedResponse {
    body: String,
    status: u16,
    fetched_at: Instant,
    ttl: Duration,
}

impl CachedResponse {
    fn is_valid(&self) -> bool {
        self.fetched_at.elapsed() < self.ttl
    }
}

// ---------------------------------------------------------------------------
// SERP link extraction
// ---------------------------------------------------------------------------

/// Domains to skip when extracting links from search result pages.
const SKIP_DOMAINS: &[&str] = &[
    "google.com",
    "google.co",
    "gstatic.com",
    "googleapis.com",
    "bing.com",
    "microsoft.com",
    "msn.com",
    "live.com",
    "duckduckgo.com",
    "brave.com",
    "schema.org",
    "w3.org",
    "youtube.com",
    "maps.google.com",
    "facebook.com",
    "instagram.com",
    "apple.com",
    "play.google.com",
];

/// Detect which search engine a URL belongs to.
fn detect_search_engine(url_str: &str) -> Option<&'static str> {
    let lower = url_str.to_lowercase();
    if lower.contains("google.com/search") {
        Some("google")
    } else if lower.contains("duckduckgo.com") {
        Some("duckduckgo")
    } else if lower.contains("bing.com/search") {
        Some("bing")
    } else {
        None
    }
}

/// Check if a URL looks like a real article (not a search engine or utility page).
fn is_article_url(url_str: &str) -> bool {
    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };

    let host = parsed.host_str().unwrap_or("").to_lowercase();

    for skip in SKIP_DOMAINS {
        if host.contains(skip) {
            return false;
        }
    }

    let path = parsed.path().to_lowercase();
    let skip_paths = [
        "/search", "/images", "/maps", "/login", "/signup", "/privacy", "/terms", "/cookie",
    ];
    if skip_paths.iter().any(|p| path.starts_with(p)) {
        return false;
    }

    let skip_exts = [
        ".pdf", ".zip", ".exe", ".dmg", ".jpg", ".png", ".gif", ".svg", ".css", ".js",
    ];
    if skip_exts.iter().any(|ext| path.ends_with(ext)) {
        return false;
    }

    true
}

/// Extract real article links from a SERP HTML page.
fn extract_serp_links(html: &str, engine: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    let max_links = 8;

    match engine {
        "google" => {
            // Google wraps result links in /url?q=<actual_url>&...
            let re = regex::Regex::new(r#"/url\?q=(https?://[^&"']+)"#).unwrap();
            for cap in re.captures_iter(html) {
                if links.len() >= max_links {
                    break;
                }
                let url = urlencoding_decode(&cap[1]);
                if is_article_url(&url) && !links.contains(&url) {
                    links.push(url);
                }
            }
        }
        "duckduckgo" => {
            // DDG uses uddg= redirect param
            let re = regex::Regex::new(r#"uddg=(https?[^&"']+)"#).unwrap();
            for cap in re.captures_iter(html) {
                if links.len() >= max_links {
                    break;
                }
                let url = urlencoding_decode(&cap[1]);
                if is_article_url(&url) && !links.contains(&url) {
                    links.push(url);
                }
            }
            // Fallback: direct hrefs
            if links.is_empty() {
                let re2 =
                    regex::Regex::new(r#"class="result__a"[^>]*href="(https?://[^"]+)""#).unwrap();
                for cap in re2.captures_iter(html) {
                    if links.len() >= max_links {
                        break;
                    }
                    let url = urlencoding_decode(&cap[1]);
                    if is_article_url(&url) && !links.contains(&url) {
                        links.push(url);
                    }
                }
            }
        }
        "bing" => {
            // Bing: article links inside <li class="b_algo">...<a href="...">
            let re =
                regex::Regex::new(r#"class="b_algo"[^>]*>.*?<a\s+href="(https?://[^"]+)""#)
                    .unwrap();
            for cap in re.captures_iter(html) {
                if links.len() >= max_links {
                    break;
                }
                let url = urlencoding_decode(&cap[1]);
                if is_article_url(&url) && !links.contains(&url) {
                    links.push(url);
                }
            }
            // Fallback
            if links.is_empty() {
                let re2 = regex::Regex::new(r#"href="(https?://[^"]+)""#).unwrap();
                for cap in re2.captures_iter(html) {
                    if links.len() >= max_links {
                        break;
                    }
                    let url = urlencoding_decode(&cap[1]);
                    if is_article_url(&url) && !links.contains(&url) {
                        links.push(url);
                    }
                }
            }
        }
        _ => {}
    }

    links
}

/// Minimal URL percent-decoding (handles %XX sequences).
fn urlencoding_decode(s: &str) -> String {
    url::form_urlencoded::parse(s.as_bytes())
        .map(|(k, v)| {
            if v.is_empty() {
                k.to_string()
            } else {
                format!("{k}={v}")
            }
        })
        .collect::<Vec<_>>()
        .join("&")
        // For simple URLs passed as values, just percent-decode directly
        .replace("%3A", ":")
        .replace("%2F", "/")
}

// ---------------------------------------------------------------------------
// Shared state for rate limiting and caching (lazy-static via Arc)
// ---------------------------------------------------------------------------

/// Global scrape state shared across all WebScrapeTool executions.
struct ScrapeState {
    /// Per-domain last-request timestamps for rate limiting.
    domain_delays: Mutex<HashMap<String, Instant>>,
    /// Per-domain backoff multipliers (grows on 429/403).
    domain_backoff: Mutex<HashMap<String, f64>>,
    /// URL-based cache with TTL.
    cache: Mutex<HashMap<String, CachedResponse>>,
}

impl ScrapeState {
    fn new() -> Self {
        Self {
            domain_delays: Mutex::new(HashMap::new()),
            domain_backoff: Mutex::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
        }
    }
}

/// Lazy global state for the scraper. Shared across all invocations.
static SCRAPE_STATE: std::sync::OnceLock<Arc<ScrapeState>> = std::sync::OnceLock::new();

fn get_scrape_state() -> Arc<ScrapeState> {
    SCRAPE_STATE
        .get_or_init(|| Arc::new(ScrapeState::new()))
        .clone()
}

// ---------------------------------------------------------------------------
// Search engine URL builders
// ---------------------------------------------------------------------------

/// Build a search engine URL for the given query.
fn build_search_url(engine: &str, query: &str, date_range: &str) -> Option<String> {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", query)
        .finish();

    match engine {
        "google" => {
            let mut url = format!("https://www.google.com/search?{encoded}");
            match date_range {
                "day" => url.push_str("&tbs=qdr:d"),
                "week" => url.push_str("&tbs=qdr:w"),
                "month" => url.push_str("&tbs=qdr:m"),
                "year" => url.push_str("&tbs=qdr:y"),
                _ => {}
            }
            Some(url)
        }
        "bing" => {
            let mut url = format!("https://www.bing.com/search?{encoded}");
            match date_range {
                "day" => url.push_str("&filters=ex1%3a%22ez1%22"),
                "week" => url.push_str("&filters=ex1%3a%22ez2%22"),
                "month" => url.push_str("&filters=ex1%3a%22ez3%22"),
                _ => {}
            }
            Some(url)
        }
        "duckduckgo" => {
            let mut url = format!("https://html.duckduckgo.com/html/?{encoded}");
            match date_range {
                "day" => url.push_str("&df=d"),
                "week" => url.push_str("&df=w"),
                "month" => url.push_str("&df=m"),
                "year" => url.push_str("&df=y"),
                _ => {}
            }
            Some(url)
        }
        _ => None,
    }
}

/// Fetch a single URL using stealth + cache + rate limiting + retry.
/// Returns (body, status, cached).
async fn fetch_single_url(
    url_str: &str,
    fingerprint: &SessionFingerprint,
    state: &ScrapeState,
    max_retries: u32,
    timeout_secs: u64,
    cache_ttl: Duration,
) -> Result<(String, u16, bool), String> {
    // Check cache
    let normalized = normalize_url(url_str);
    {
        let cache = state.cache.lock().await;
        if let Some(cached) = cache.get(&normalized) {
            if cached.is_valid() {
                return Ok((cached.body.clone(), cached.status, true));
            }
        }
    }

    // Rate limiting
    let domain = url::Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
        .unwrap_or_default();
    {
        let mut delays = state.domain_delays.lock().await;
        let backoffs = state.domain_backoff.lock().await;
        let base_delay = Duration::from_secs(1);
        if let Some(last) = delays.get(&domain) {
            let backoff_multiplier = backoffs.get(&domain).copied().unwrap_or(1.0);
            let required_delay = base_delay.mul_f64(backoff_multiplier);
            let elapsed = last.elapsed();
            if elapsed < required_delay {
                tokio::time::sleep(apply_jitter(required_delay - elapsed)).await;
            }
        }
        delays.insert(domain.clone(), Instant::now());
    }

    // Build client
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let headers = fingerprint.build_headers();

    // Fetch with retry
    let mut last_status: u16 = 0;
    let mut last_body = String::new();
    let mut success = false;

    for attempt in 0..=max_retries {
        let result = client.get(url_str).headers(headers.clone()).send().await;
        match result {
            Ok(response) => {
                last_status = response.status().as_u16();
                let should_retry = matches!(last_status, 429 | 500 | 502 | 503 | 504);
                if last_status == 429 || last_status == 403 {
                    let mut backoffs = state.domain_backoff.lock().await;
                    let current = backoffs.get(&domain).copied().unwrap_or(1.0);
                    backoffs.insert(domain.clone(), (current * 2.0).min(30.0));
                }
                if should_retry && attempt < max_retries {
                    let base = Duration::from_secs(1u64 << attempt.min(4));
                    tokio::time::sleep(apply_jitter(base)).await;
                    continue;
                }
                last_body = response.text().await.unwrap_or_default();
                success = (200..400).contains(&(last_status as i32));
                break;
            }
            Err(_) if attempt < max_retries => {
                let base = Duration::from_secs(1u64 << attempt.min(4));
                tokio::time::sleep(apply_jitter(base)).await;
                continue;
            }
            Err(e) => {
                return Err(format!("HTTP request failed after {max_retries} retries: {e}"));
            }
        }
    }

    // Cache successful response
    if success {
        let mut cache = state.cache.lock().await;
        cache.insert(normalized, CachedResponse {
            body: last_body.clone(),
            status: last_status,
            fetched_at: Instant::now(),
            ttl: cache_ttl,
        });
    }

    Ok((last_body, last_status, false))
}

// ---------------------------------------------------------------------------
// WebScrapeTool struct and factory
// ---------------------------------------------------------------------------

data_tool! {
    struct WebScrapeTool, factory WebScrapeFactory;
    tool_type = "data/web_scrape",
    name = "Web Scrape",
    description = "Searches the web or fetches URLs. Supports query-based search (Google/Bing/DuckDuckGo) and direct URL scraping.",
    inputs = [
        field("query", FieldType::String, false, "Search query (searches Google/Bing/DuckDuckGo)"),
        field("url", FieldType::String, false, "Direct URL to fetch (alternative to query)"),
    ],
    outputs = [
        field("results", FieldType::Array, true, "Array of {url, title, content, status_code, success, source}"),
        field("content", FieldType::String, false, "Page content (single URL mode)"),
        field("status", FieldType::Number, false, "HTTP status code (single URL mode)"),
        field("url", FieldType::String, false, "URL fetched (single URL mode)"),
        field("cached", FieldType::Boolean, false, "Whether the response came from cache"),
        field("links", FieldType::Array, false, "Extracted links if URL is a SERP"),
    ],
    config_fields = [
        field("search_engines", FieldType::String, false, "Comma-separated engines: google,bing,duckduckgo (default google)"),
        field("max_results_per_query", FieldType::Number, false, "Max results per search engine (default 5)"),
        field("max_content_length", FieldType::Number, false, "Max chars per result content (default 10000)"),
        field("date_range", FieldType::String, false, "Date range filter: day, week, month, year"),
        field("max_retries", FieldType::Number, false, "Max retries on failure (default 3)"),
        field("timeout_seconds", FieldType::Number, false, "Request timeout in seconds (default 30)"),
        field("cache_ttl_seconds", FieldType::Number, false, "Cache TTL in seconds (default 300)"),
        field("output_schema", FieldType::String, false, "JSON schema for LLM-based structured extraction"),
        field("extract_links", FieldType::Boolean, false, "Whether to extract links from SERP pages (default false)"),
    ]
}

#[async_trait]
impl Tool for WebScrapeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let query = inputs.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let url_input = inputs.get("url").and_then(|v| v.as_str()).unwrap_or("");

        if query.is_empty() && url_input.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "data/web_scrape".into(),
                message: "Either 'query' or 'url' input is required".into(),
            });
        }

        // Read config
        let max_retries = config.get("max_retries").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
        let timeout_secs = config.get("timeout_seconds").and_then(|v| v.as_u64()).unwrap_or(30);
        let cache_ttl_secs = config.get("cache_ttl_seconds").and_then(|v| v.as_u64()).unwrap_or(300);
        let cache_ttl = Duration::from_secs(cache_ttl_secs);
        let max_results = config.get("max_results_per_query").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let max_content_len = config.get("max_content_length").and_then(|v| v.as_u64()).unwrap_or(10000) as usize;
        let date_range = config.get("date_range").and_then(|v| v.as_str()).unwrap_or("");
        let engines_str = config.get("search_engines").and_then(|v| v.as_str()).unwrap_or("google");
        let extract_links = config.get("extract_links").and_then(|v| v.as_bool()).unwrap_or(false);
        let output_schema = config.get("output_schema").and_then(|v| v.as_str()).unwrap_or("").to_string();

        let state = get_scrape_state();
        let session_id = context.session_id();
        let fingerprint = SessionFingerprint::generate(session_id);

        // ---------------------------------------------------------------
        // MODE 1: Query-based search (search engines -> extract links -> fetch)
        // ---------------------------------------------------------------
        if !query.is_empty() {
            let engines: Vec<&str> = engines_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            let mut all_results: Vec<Value> = Vec::new();

            for engine in &engines {
                let search_url = match build_search_url(engine, query, date_range) {
                    Some(u) => u,
                    None => continue,
                };

                // Fetch the SERP page
                let (serp_body, serp_status, _cached) = match fetch_single_url(
                    &search_url, &fingerprint, &state, max_retries, timeout_secs, cache_ttl,
                ).await {
                    Ok(r) => r,
                    Err(_) => continue, // skip this engine on error
                };

                if !(200..400).contains(&(serp_status as i32)) {
                    continue;
                }

                // Extract real article links from SERP
                let links = extract_serp_links(&serp_body, engine);
                let links_to_fetch: Vec<&str> = links.iter().map(|s| s.as_str()).take(max_results).collect();

                // Fetch each result page
                for link in links_to_fetch {
                    let (body, status, cached) = match fetch_single_url(
                        link, &fingerprint, &state, max_retries, timeout_secs, cache_ttl,
                    ).await {
                        Ok(r) => r,
                        Err(_) => (String::new(), 0u16, false),
                    };

                    let success = (200..400).contains(&(status as i32));
                    let truncated = if body.len() > max_content_len {
                        &body[..max_content_len]
                    } else {
                        &body
                    };

                    // Try to extract a title from <title> tag
                    let title = regex::Regex::new(r"(?i)<title[^>]*>(.*?)</title>")
                        .ok()
                        .and_then(|re| re.captures(truncated))
                        .map(|c| c[1].trim().to_string())
                        .unwrap_or_default();

                    all_results.push(json!({
                        "url": link,
                        "title": title,
                        "content": truncated,
                        "status_code": status,
                        "success": success,
                        "cached": cached,
                        "source": *engine,
                    }));
                }
            }

            let mut out = HashMap::new();
            out.insert("results".to_string(), json!(all_results));
            return Ok(out);
        }

        // ---------------------------------------------------------------
        // MODE 2: Direct URL fetch
        // ---------------------------------------------------------------
        let (last_body, last_status, cached) = fetch_single_url(
            url_input, &fingerprint, &state, max_retries, timeout_secs, cache_ttl,
        ).await.map_err(|e| ToolError::ExecutionFailed {
            tool_type: "data/web_scrape".into(),
            message: e,
        })?;

        let success = (200..400).contains(&(last_status as i32));

        // SERP link extraction
        let mut extracted_links: Option<Vec<String>> = None;
        if extract_links || detect_search_engine(url_input).is_some() {
            if let Some(engine) = detect_search_engine(url_input) {
                let links = extract_serp_links(&last_body, engine);
                if !links.is_empty() {
                    extracted_links = Some(links);
                }
            }
        }

        // LLM-based structured extraction
        if !output_schema.is_empty() && success {
            let llm = context.llm();
            let body_slice = &last_body[..last_body.len().min(8000)];
            let prompt = format!(
                "Extract the following fields from the content below. \
                 Return ONLY valid JSON matching this schema: {output_schema}\n\n\
                 Content:\n{body_slice}\n\nJSON output:",
            );
            if let Ok(llm_resp) = llm.call("", &prompt, &[], 0.0, 2000).await {
                let re = regex::Regex::new(r"\{[^{}]*\}").unwrap();
                if let Some(m) = re.find(&llm_resp.response) {
                    if let Ok(extracted) = serde_json::from_str::<Value>(m.as_str()) {
                        let mut out = HashMap::new();
                        out.insert("content".to_string(), json!(last_body));
                        out.insert("status".to_string(), json!(last_status));
                        out.insert("url".to_string(), json!(url_input));
                        out.insert("cached".to_string(), json!(cached));
                        out.insert("extracted".to_string(), extracted);
                        let truncated = &last_body[..last_body.len().min(max_content_len)];
                        out.insert("results".to_string(), json!([{
                            "url": url_input, "title": "", "content": truncated,
                            "status_code": last_status, "success": success, "source": "direct",
                        }]));
                        if let Some(links) = extracted_links {
                            out.insert("links".to_string(), json!(links));
                        }
                        return Ok(out);
                    }
                }
            }
        }

        // Build response
        let truncated = &last_body[..last_body.len().min(max_content_len)];
        let mut out = HashMap::new();
        out.insert("content".to_string(), json!(truncated));
        out.insert("status".to_string(), json!(last_status));
        out.insert("url".to_string(), json!(url_input));
        out.insert("cached".to_string(), json!(cached));
        out.insert("results".to_string(), json!([{
            "url": url_input, "title": "", "content": truncated,
            "status_code": last_status, "success": success, "source": "direct",
        }]));
        if let Some(links) = extracted_links {
            out.insert("links".to_string(), json!(links));
        }

        Ok(out)
    }
}

// ===========================================================================
// HtmlToMarkdownTool
// ===========================================================================

data_tool! {
    struct HtmlToMarkdownTool, factory HtmlToMarkdownFactory;
    tool_type = "data/html_to_markdown",
    name = "HTML to Markdown",
    description = "Converts HTML content to clean Markdown by stripping tags and converting semantic elements",
    inputs = [
        field("html", FieldType::String, true, "HTML content to convert"),
    ],
    outputs = [
        field("markdown", FieldType::String, true, "Converted markdown text"),
        field("length", FieldType::Number, true, "Length of markdown output"),
    ],
    config_fields = []
}

/// Simple HTML to Markdown converter.
/// Strips script/style tags, converts headings, links, paragraphs, and strips remaining tags.
fn html_to_markdown(html: &str) -> String {
    use regex::Regex;

    let mut text = html.to_string();

    // Remove script and style blocks
    let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    text = script_re.replace_all(&text, "").to_string();
    let style_re = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    text = style_re.replace_all(&text, "").to_string();
    let noscript_re = Regex::new(r"(?is)<noscript[^>]*>.*?</noscript>").unwrap();
    text = noscript_re.replace_all(&text, "").to_string();

    // Convert headings: <h1>text</h1> -> # text
    for level in 1..=6 {
        let hashes = "#".repeat(level);
        let re = Regex::new(&format!(r"(?is)<h{level}[^>]*>(.*?)</h{level}>")).unwrap();
        text = re
            .replace_all(&text, |caps: &regex::Captures| {
                format!("\n\n{} {}\n\n", hashes, caps[1].trim())
            })
            .to_string();
    }

    // Convert links: <a href="url">text</a> -> [text](url)
    let link_re = Regex::new(r#"(?is)<a[^>]*href\s*=\s*["']([^"']*)["'][^>]*>(.*?)</a>"#).unwrap();
    text = link_re
        .replace_all(&text, |caps: &regex::Captures| {
            let href = &caps[1];
            let link_text = caps[2].trim();
            if link_text.is_empty() {
                String::new()
            } else {
                format!("[{}]({})", link_text, href)
            }
        })
        .to_string();

    // Convert strong/bold
    let bold_re = Regex::new(r"(?is)<(?:strong|b)[^>]*>(.*?)</(?:strong|b)>").unwrap();
    text = bold_re
        .replace_all(&text, |caps: &regex::Captures| {
            format!("**{}**", caps[1].trim())
        })
        .to_string();

    // Convert emphasis/italic
    let em_re = Regex::new(r"(?is)<(?:em|i)[^>]*>(.*?)</(?:em|i)>").unwrap();
    text = em_re
        .replace_all(&text, |caps: &regex::Captures| {
            format!("*{}*", caps[1].trim())
        })
        .to_string();

    // Convert list items
    let li_re = Regex::new(r"(?is)<li[^>]*>(.*?)</li>").unwrap();
    text = li_re
        .replace_all(&text, |caps: &regex::Captures| {
            format!("\n- {}", caps[1].trim())
        })
        .to_string();

    // Convert paragraphs and divs to double newlines
    let p_re = Regex::new(r"(?is)<(?:p|div)[^>]*>").unwrap();
    text = p_re.replace_all(&text, "\n\n").to_string();
    let p_close_re = Regex::new(r"(?is)</(?:p|div)>").unwrap();
    text = p_close_re.replace_all(&text, "\n\n").to_string();

    // Convert <br> to newlines
    let br_re = Regex::new(r"(?i)<br\s*/?>").unwrap();
    text = br_re.replace_all(&text, "\n").to_string();

    // Convert <hr> to horizontal rules
    let hr_re = Regex::new(r"(?i)<hr\s*/?>").unwrap();
    text = hr_re.replace_all(&text, "\n\n---\n\n").to_string();

    // Strip all remaining HTML tags
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    text = tag_re.replace_all(&text, "").to_string();

    // Decode common HTML entities
    text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Clean up excessive blank lines
    let multi_newline = Regex::new(r"\n{3,}").unwrap();
    text = multi_newline.replace_all(&text, "\n\n").to_string();

    text.trim().to_string()
}

#[async_trait]
impl Tool for HtmlToMarkdownTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let html = inputs
            .get("html")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/html_to_markdown".into(),
                message: "input 'html' is required".into(),
            })?;

        let markdown = html_to_markdown(html);
        let length = markdown.len();

        let mut out = HashMap::new();
        out.insert("markdown".to_string(), json!(markdown));
        out.insert("length".to_string(), json!(length));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all data tools into the given registry.
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
    registry.register("data/html_to_markdown", Box::new(HtmlToMarkdownFactory::new()));
    registry.register("data/rag_search", Box::new(RagSearchFactory::new()));
}

// ===========================================================================
// RagSearchTool — semantic search with REAL embeddings
// ===========================================================================

data_tool! {
    struct RagSearchTool, factory RagSearchFactory;
    tool_type = "data/rag_search",
    name = "RAG Search",
    description = "Semantic search over documents using real embeddings. Chunks documents, generates embeddings via LLM, returns top-K by cosine similarity.",
    inputs = [
        field("query", FieldType::String, true, "Search query"),
        field("documents", FieldType::Array, false, "Array of text documents to search. If not provided, uses config."),
    ],
    outputs = [
        field("results", FieldType::Array, true, "Ranked chunks with scores"),
        field("chunks_total", FieldType::Number, true, "Total chunks generated"),
        field("embedding_dimensions", FieldType::Number, true, "Embedding vector dimensions"),
    ],
    config_fields = [
        field("documents", FieldType::Array, false, "Static documents to search (alternative to input)"),
        field("top_k", FieldType::Number, false, "Number of results (default 3)"),
        field("chunk_strategy", FieldType::String, false, "Chunking: paragraph, sentence, fixed_size (default paragraph)"),
        field("chunk_size", FieldType::Number, false, "Max chars per chunk (default 512)"),
        field("embedding_model", FieldType::String, false, "Embedding model (default: provider default)"),
    ]
}

#[async_trait]
impl Tool for RagSearchTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let query = inputs.get("query")
            .and_then(|v| v.as_str())
            .or_else(|| config.get("query").and_then(|v| v.as_str()))
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/rag_search".into(),
                message: "input 'query' is required".into(),
            })?;

        // Get documents from input or config.
        let docs: Vec<String> = inputs.get("documents")
            .or_else(|| config.get("documents"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if docs.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "data/rag_search".into(),
                message: "documents are required (via input or config)".into(),
            });
        }

        let top_k = config.get("top_k").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
        let chunk_strategy = config.get("chunk_strategy").and_then(|v| v.as_str()).unwrap_or("paragraph");
        let chunk_size = config.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(512) as usize;
        let embed_model = config.get("embedding_model").and_then(|v| v.as_str()).unwrap_or("");

        // Chunk documents.
        let rag_config = crate::rag::RAGPipelineConfig {
            name: "tool".into(),
            source_type: crate::rag::SourceType::Text,
            chunking_strategy: match chunk_strategy {
                "sentence" => crate::rag::ChunkingStrategy::Sentence,
                "fixed_size" => crate::rag::ChunkingStrategy::FixedSize,
                _ => crate::rag::ChunkingStrategy::Paragraph,
            },
            chunk_size,
            chunk_overlap: 50,
            embedding_model: embed_model.to_string(),
        };

        let mut all_chunks: Vec<String> = Vec::new();
        for doc in &docs {
            all_chunks.extend(crate::rag::chunk_text(doc, &rag_config));
        }

        if all_chunks.is_empty() {
            let mut out = HashMap::new();
            out.insert("results".to_string(), json!([]));
            out.insert("chunks_total".to_string(), json!(0));
            out.insert("embedding_dimensions".to_string(), json!(0));
            return Ok(out);
        }

        // Generate REAL embeddings via context.llm().embed().
        let query_emb = context.llm().embed(query, embed_model).await.map_err(|e| {
            ToolError::ExecutionFailed {
                tool_type: "data/rag_search".into(),
                message: format!("Failed to embed query: {e}"),
            }
        })?;

        let mut chunk_embs = Vec::new();
        for chunk in &all_chunks {
            let emb = context.llm().embed(chunk, embed_model).await.map_err(|e| {
                ToolError::ExecutionFailed {
                    tool_type: "data/rag_search".into(),
                    message: format!("Failed to embed chunk: {e}"),
                }
            })?;
            chunk_embs.push(emb);
        }

        // Cosine similarity ranking.
        let mut scored: Vec<(usize, f64)> = chunk_embs.iter().enumerate().map(|(i, emb)| {
            let dot: f64 = query_emb.iter().zip(emb.iter()).map(|(a, b)| a * b).sum();
            let mag_a: f64 = query_emb.iter().map(|x| x * x).sum::<f64>().sqrt();
            let mag_b: f64 = emb.iter().map(|x| x * x).sum::<f64>().sqrt();
            let sim = if mag_a > 0.0 && mag_b > 0.0 { dot / (mag_a * mag_b) } else { 0.0 };
            (i, sim)
        }).collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<Value> = scored.iter().take(top_k).map(|(i, score)| {
            json!({"chunk": all_chunks[*i], "score": score, "index": i})
        }).collect();

        let dims = query_emb.len();

        let mut out = HashMap::new();
        out.insert("results".to_string(), json!(results));
        out.insert("chunks_total".to_string(), json!(all_chunks.len()));
        out.insert("embedding_dimensions".to_string(), json!(dims));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        async fn execute(
            &self,
            _query: &str,
            _params: &[Value],
        ) -> Result<Value, ResourceError> {
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
            data.insert(
                "test/file.txt".to_string(),
                b"hello world".to_vec(),
            );
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
                tokens_used: TokenUsage { input: 0, output: 0 },
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
        config.insert("query".to_string(), json!("SELECT * FROM users WHERE id = 1"));
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
        config.insert("query".to_string(), json!("SELECT * FROM users WHERE id = 999"));
        config.insert("mode".to_string(), json!("one"));

        let result = tool.execute(inputs, &config, &ctx).await.unwrap();
        assert_eq!(result["count"], json!(0));
        assert!(result["row"].is_null());
    }

    #[tokio::test]
    async fn db_read_no_db_fails() {
        let ctx = TestContext::empty();
        let tool = DbReadTool;
        let result = tool
            .execute(HashMap::new(), &HashMap::new(), &ctx)
            .await;
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
        let result = tool
            .execute(HashMap::new(), &HashMap::new(), &ctx)
            .await;
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
        let result = tool
            .execute(HashMap::new(), &HashMap::new(), &ctx)
            .await;
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
        let result = tool
            .execute(inputs, &HashMap::new(), &ctx)
            .await
            .unwrap();
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
        let result = tool
            .execute(inputs, &HashMap::new(), &ctx)
            .await
            .unwrap();
        let md = result["markdown"].as_str().unwrap();
        assert!(md.contains("# Title"));
        assert!(md.contains("**world**"));
        assert!(result["length"].as_u64().unwrap() > 0);
    }

    #[test]
    fn html_to_markdown_strips_script() {
        let md = super::html_to_markdown("<p>hello</p><script>evil()</script><p>world</p>");
        assert!(!md.contains("evil"));
        assert!(md.contains("hello"));
        assert!(md.contains("world"));
    }

    #[test]
    fn html_to_markdown_converts_links() {
        let md = super::html_to_markdown(r#"<a href="https://example.com">Click here</a>"#);
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
                ms >= 700 && ms <= 1300,
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
