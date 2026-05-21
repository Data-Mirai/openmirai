use std::collections::HashMap;

use async_trait::async_trait;
use regex;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{ToolField, ToolSpec};
use crate::tools::registry::{Tool, ToolFactory, ToolRegistry};

// ---------------------------------------------------------------------------
// Helper: field builder (same pattern as logic.rs)
// ---------------------------------------------------------------------------

fn field(name: &str, field_type: &str, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type: field_type.into(),
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
            fn create(&self) -> Box<dyn Tool> {
                Box::new($tool)
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
        field("query_params", "object", false, "Filter parameters: {column: value} for WHERE clause"),
    ],
    outputs = [
        field("rows", "array", false, "Array of matched rows (mode=all)"),
        field("row", "object", false, "Single matched row (mode=one)"),
        field("count", "number", true, "Number of rows returned"),
    ],
    config_fields = [
        field("query", "string", true, "SQL query to execute"),
        field("mode", "string", false, "Read mode: 'one' or 'all' (default: all)"),
    ]
}

#[async_trait]
impl Tool for DbReadTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
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
        field("data", "object", true, "Row data to write"),
    ],
    outputs = [
        field("table", "string", true, "Target table name"),
        field("action", "string", true, "Action performed: inserted or updated"),
        field("id", "string", false, "Row ID of the written record"),
    ],
    config_fields = [
        field("table", "string", true, "Target table name"),
        field("mode", "string", false, "Write mode: 'insert' or 'upsert' (default: insert)"),
    ]
}

#[async_trait]
impl Tool for DbWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
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
        field("path", "string", true, "Storage path/key to read"),
    ],
    outputs = [
        field("content", "string", false, "File content as text"),
        field("path", "string", true, "Path that was read"),
        field("found", "boolean", true, "Whether the file was found"),
    ],
    config_fields = [
        field("mode", "string", false, "Read mode: 'read' or 'presign' (default: read)"),
    ]
}

#[async_trait]
impl Tool for StorageReadTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
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
        field("path", "string", true, "Storage path/key to write to"),
        field("content", "string", true, "Content to write"),
    ],
    outputs = [
        field("path", "string", true, "Path that was written"),
        field("bytes_written", "number", true, "Number of bytes written"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for StorageWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
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
        field("query", "string", false, "Search query for vault notes"),
        field("path", "string", false, "Specific vault path to read"),
    ],
    outputs = [
        field("notes", "array", true, "Matched vault notes"),
        field("count", "number", true, "Number of notes returned"),
    ],
    config_fields = [
        field("folder", "string", false, "Vault folder to search in"),
        field("limit", "number", false, "Maximum notes to return"),
    ]
}

#[async_trait]
impl Tool for VaultReadTool {
    async fn execute(
        &self,
        _inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
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
        field("path", "string", true, "Vault path for the note"),
        field("title", "string", false, "Note title"),
        field("content", "string", true, "Note content"),
    ],
    outputs = [
        field("path", "string", true, "Path where note was written"),
        field("written", "boolean", true, "Whether write succeeded"),
    ],
    config_fields = [
        field("tags", "string", false, "Comma-separated tags"),
    ]
}

#[async_trait]
impl Tool for VaultWriteTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
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
        field("entity_type", "string", true, "Entity type to query"),
        field("filters", "object", false, "Filter conditions as {field: value}"),
    ],
    outputs = [
        field("entities", "array", true, "Matched entities"),
        field("count", "number", true, "Number of entities returned"),
    ],
    config_fields = [
        field("limit", "number", false, "Maximum entities to return"),
        field("order_by", "string", false, "Field to order by"),
    ]
}

#[async_trait]
impl Tool for EntityQueryTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
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
        field("entity_type", "string", true, "Entity type"),
        field("data", "object", true, "Entity field data"),
        field("id", "string", false, "Entity ID (if updating)"),
    ],
    outputs = [
        field("id", "string", true, "Entity ID"),
        field("action", "string", true, "Action performed: created or updated"),
    ],
    config_fields = []
}

#[async_trait]
impl Tool for EntityUpsertTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        _config: HashMap<String, Value>,
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
// WebScrapeTool
// ===========================================================================

data_tool! {
    struct WebScrapeTool, factory WebScrapeFactory;
    tool_type = "data/web_scrape",
    name = "Web Scrape",
    description = "Fetches web pages via HTTP GET and returns their content",
    inputs = [
        field("url", "string", true, "URL to fetch"),
    ],
    outputs = [
        field("content", "string", true, "Page content as text"),
        field("status", "number", true, "HTTP status code"),
        field("url", "string", true, "URL that was fetched"),
    ],
    config_fields = [
        field("timeout", "number", false, "Request timeout in seconds"),
    ]
}

#[async_trait]
impl Tool for WebScrapeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let url = inputs
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool_type: "data/web_scrape".into(),
                message: "input 'url' is required".into(),
            })?;

        let timeout_secs = config
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/web_scrape".into(),
                message: format!("Failed to build HTTP client: {e}"),
            })?;

        let response = client
            .get(url)
            .header("User-Agent", "DataMirai-Engine/1.0")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/web_scrape".into(),
                message: format!("HTTP request failed: {e}"),
            })?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_type: "data/web_scrape".into(),
                message: format!("Failed to read response body: {e}"),
            })?;

        let mut out = HashMap::new();
        out.insert("content".to_string(), json!(body));
        out.insert("status".to_string(), json!(status));
        out.insert("url".to_string(), json!(url));
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
        field("html", "string", true, "HTML content to convert"),
    ],
    outputs = [
        field("markdown", "string", true, "Converted markdown text"),
        field("length", "number", true, "Length of markdown output"),
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
        _config: HashMap<String, Value>,
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
        assert_eq!(result["count"], json!(0));
        assert!(result["row"].is_null());
    }

    #[tokio::test]
    async fn db_read_no_db_fails() {
        let ctx = TestContext::empty();
        let tool = DbReadTool;
        let result = tool
            .execute(HashMap::new(), HashMap::new(), &ctx)
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
        assert_eq!(result["table"], json!("users"));
        assert_eq!(result["action"], json!("inserted"));
    }

    #[tokio::test]
    async fn db_write_no_db_fails() {
        let ctx = TestContext::empty();
        let tool = DbWriteTool;
        let result = tool
            .execute(HashMap::new(), HashMap::new(), &ctx)
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
        assert_eq!(result["found"], json!(true));
    }

    #[tokio::test]
    async fn storage_read_no_path_fails() {
        let ctx = TestContext::with_storage();
        let tool = StorageReadTool;
        let result = tool
            .execute(HashMap::new(), HashMap::new(), &ctx)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn storage_read_no_storage_fails() {
        let ctx = TestContext::empty();
        let tool = StorageReadTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("test.txt"));
        let result = tool.execute(inputs, HashMap::new(), &ctx).await;
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

        let result = tool.execute(inputs, config, &ctx).await.unwrap();
        assert_eq!(result["path"], json!("output/result.txt"));
        assert_eq!(result["bytes_written"], json!(15)); // "Hello, storage!" is 15 bytes
    }

    #[tokio::test]
    async fn storage_write_missing_path_fails() {
        let ctx = TestContext::with_storage();
        let tool = StorageWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("content".to_string(), json!("data"));
        let result = tool.execute(inputs, HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn storage_write_missing_content_fails() {
        let ctx = TestContext::with_storage();
        let tool = StorageWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("output.txt"));
        let result = tool.execute(inputs, HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn storage_write_no_storage_fails() {
        let ctx = TestContext::empty();
        let tool = StorageWriteTool;
        let mut inputs = HashMap::new();
        inputs.insert("path".to_string(), json!("out.txt"));
        inputs.insert("content".to_string(), json!("data"));
        let result = tool.execute(inputs, HashMap::new(), &ctx).await;
        assert!(result.is_err());
    }

    // -- VaultReadTool --------------------------------------------------------

    #[tokio::test]
    async fn vault_read_returns_empty() {
        let ctx = TestContext::empty();
        let tool = VaultReadTool;
        let result = tool
            .execute(HashMap::new(), HashMap::new(), &ctx)
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
            .execute(inputs, HashMap::new(), &ctx)
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
            .execute(inputs, HashMap::new(), &ctx)
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
        assert_eq!(reg.list_tools().len(), 10);
    }
}
