//! Network tools — generic HTTP requests as graph nodes.
//!
//! `net/http_request` is the reusable primitive for *writing* to any HTTP API
//! from inside a workflow: it POSTs (by default) a JSON body to a URL. This is
//! how a workflow becomes a traceable SOP that talks to the engine's own API —
//! e.g. writing the fleet SoT via `POST /api/v1/fleet/status` after spawning a
//! session.
//!
//! Both `url` and the string leaves of `body` support `${var}` placeholders
//! that are substituted from the node's resolved inputs (values wired in from
//! upstream nodes via edge `data_map`). So a spawn node can hand its `id` to a
//! downstream fleet-write node with `data_map: { session_id: "spawn.id" }` and
//! a body of `{ "id": "${session_id}" }`.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use crate::core::context::ExecutionContext;
use crate::core::runner::ToolError;
use crate::tools::base::{field, FieldType};
use crate::tools::registry::{Tool, ToolRegistry};

/// `${var}` where var is a bare identifier (no dot — dotted refs belong to the
/// runner's data_map layer, which resolves BEFORE the tool runs).
static VAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([a-zA-Z0-9_]+)\}").expect("valid regex"));

crate::define_tool! {
    struct HttpRequestTool, factory HttpRequestFactory;
    tool_type = "net/http_request",
    name = "HTTP Request",
    description = "Makes an HTTP request (default POST) to a URL with an optional JSON body. Substitutes ${var} placeholders in url/body from the node's inputs. Reusable node to call any HTTP API — e.g. write the fleet SoT via POST /api/v1/fleet/status.",
    category = "net",
    inputs = [
        field("url", FieldType::String, false, "Full or relative URL (relative joins base_url). Supports ${var}."),
        field("body", FieldType::Any, false, "JSON body; string leaves support ${var} interpolation."),
        field("base_url", FieldType::String, false, "Prefix for a relative url. Supports ${var}."),
    ],
    outputs = [
        field("ok", FieldType::Boolean, true, "True when the HTTP status is 2xx."),
        field("http_status", FieldType::Number, true, "HTTP status code."),
        field("response", FieldType::Any, false, "Parsed JSON response body (if JSON). Its top-level keys are also flattened into the node output so ${node.field} resolves."),
        field("body_text", FieldType::String, false, "Raw response body text."),
    ],
    config_fields = [
        field("url", FieldType::String, false, "Full or relative URL. Supports ${var}."),
        field("method", FieldType::String, false, "HTTP method (default POST)."),
        field("base_url", FieldType::String, false, "Prefix for a relative url. Supports ${var}."),
        field("headers", FieldType::Object, false, "Extra request headers (string values)."),
        field("body", FieldType::Any, false, "JSON body template."),
        field("timeout_seconds", FieldType::Number, false, "Request timeout in seconds (default 30)."),
        field("api_key", FieldType::String, false, "If set, sent as the X-API-Key header. Supports ${var}."),
        field("ignore_http_errors", FieldType::Boolean, false, "If true, a >=400 status returns ok=false instead of failing the node (default false)."),
    ]
}

/// Read a field preferring the resolved `inputs`, falling back to static
/// `config` (config is also merged into inputs by the runner, but reading
/// config directly keeps the structural template intact).
fn pick<'a>(
    key: &str,
    inputs: &'a HashMap<String, Value>,
    config: &'a HashMap<String, Value>,
) -> Option<&'a Value> {
    config.get(key).or_else(|| inputs.get(key))
}

/// Substitute `${var}` occurrences inside a string using `inputs`. Unknown vars
/// are left untouched.
fn interp_str(s: &str, inputs: &HashMap<String, Value>) -> String {
    VAR_RE
        .replace_all(s, |caps: &regex::Captures| match inputs.get(&caps[1]) {
            Some(Value::String(v)) => v.clone(),
            Some(other) => other.to_string(),
            None => caps[0].to_string(),
        })
        .into_owned()
}

/// Recursively interpolate a JSON value. A leaf that is EXACTLY `${var}` and
/// whose value is non-string keeps its original JSON type (so numbers/bools
/// survive); otherwise string substitution applies.
fn interp_val(v: &Value, inputs: &HashMap<String, Value>) -> Value {
    match v {
        Value::String(s) => {
            if let Some(caps) = VAR_RE.captures(s) {
                if caps.get(0).map(|m| m.as_str()) == Some(s.as_str()) {
                    if let Some(val) = inputs.get(&caps[1]) {
                        return val.clone();
                    }
                }
            }
            Value::String(interp_str(s, inputs))
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| interp_val(x, inputs)).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, x)| (k.clone(), interp_val(x, inputs))).collect())
        }
        other => other.clone(),
    }
}

#[async_trait]
impl Tool for HttpRequestTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        _context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let err = |m: String| ToolError::ExecutionFailed {
            tool_type: "net/http_request".into(),
            message: m,
        };

        let method = pick("method", &inputs, config)
            .and_then(|v| v.as_str())
            .unwrap_or("POST")
            .to_uppercase();

        let raw_url = pick("url", &inputs, config)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if raw_url.trim().is_empty() {
            return Err(err("url is required".into()));
        }
        let url = interp_str(&raw_url, &inputs);
        let full_url = if url.starts_with("http://") || url.starts_with("https://") {
            url
        } else {
            let base = pick("base_url", &inputs, config)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let base = interp_str(base, &inputs);
            if base.is_empty() {
                return Err(err(format!(
                    "url '{url}' is relative but no base_url was provided"
                )));
            }
            format!("{}/{}", base.trim_end_matches('/'), url.trim_start_matches('/'))
        };

        let body = pick("body", &inputs, config).map(|b| interp_val(b, &inputs));
        let timeout_secs = pick("timeout_seconds", &inputs, config)
            .and_then(|v| v.as_u64())
            .unwrap_or(30);
        let ignore_http_errors = pick("ignore_http_errors", &inputs, config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| err(format!("invalid HTTP method '{method}'")))?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| err(format!("failed to build HTTP client: {e}")))?;

        let mut rb = client.request(method, &full_url);
        if let Some(b) = &body {
            rb = rb.json(b);
        }
        // Custom headers.
        if let Some(Value::Object(map)) = pick("headers", &inputs, config) {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    rb = rb.header(k.as_str(), interp_str(s, &inputs));
                }
            }
        }
        if let Some(key) = pick("api_key", &inputs, config).and_then(|v| v.as_str()) {
            rb = rb.header("X-API-Key", interp_str(key, &inputs));
        }

        let resp = rb
            .send()
            .await
            .map_err(|e| err(format!("request to {full_url} failed: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        let ok = (200..300).contains(&status);

        if !ok && !ignore_http_errors {
            return Err(err(format!(
                "HTTP {status} from {full_url}: {}",
                text.chars().take(300).collect::<String>()
            )));
        }

        // Flatten a JSON-object response's top-level keys into the node output
        // so downstream nodes can reference `${this_node.field}` (e.g. spawn.id).
        let mut out = HashMap::new();
        if let Some(Value::Object(map)) = &parsed {
            for (k, v) in map {
                out.insert(k.clone(), v.clone());
            }
        }
        out.insert("http_status".to_string(), json!(status));
        out.insert("ok".to_string(), json!(ok));
        out.insert("body_text".to_string(), json!(text));
        if let Some(p) = parsed {
            out.insert("response".to_string(), p);
        }
        Ok(out)
    }
}

/// Register all network tools into the given registry.
pub fn register_net_tools(registry: &mut ToolRegistry) {
    registry.register("net/http_request", Box::new(HttpRequestFactory::new()));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::InMemoryContext;
    use serde_json::json;

    fn ctx() -> InMemoryContext {
        InMemoryContext::new("test-run")
    }

    #[test]
    fn interpolates_string_placeholders() {
        let mut inputs = HashMap::new();
        inputs.insert("id".to_string(), json!("abc123"));
        inputs.insert("port".to_string(), json!(4321));
        assert_eq!(interp_str("http://h:${port}/x/${id}", &inputs), "http://h:4321/x/abc123");
        // Unknown var left as-is.
        assert_eq!(interp_str("${missing}", &inputs), "${missing}");
    }

    #[test]
    fn exact_placeholder_preserves_type() {
        let mut inputs = HashMap::new();
        inputs.insert("n".to_string(), json!(42));
        inputs.insert("flag".to_string(), json!(true));
        let body = json!({"count": "${n}", "on": "${flag}", "label": "n=${n}"});
        let out = interp_val(&body, &inputs);
        assert_eq!(out["count"], json!(42), "exact numeric placeholder keeps type");
        assert_eq!(out["on"], json!(true));
        assert_eq!(out["label"], json!("n=42"), "embedded placeholder stringifies");
    }

    #[tokio::test]
    async fn missing_url_errors() {
        let tool = HttpRequestTool;
        let res = tool.execute(HashMap::new(), &HashMap::new(), &ctx()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn relative_url_without_base_errors() {
        let tool = HttpRequestTool;
        let mut config = HashMap::new();
        config.insert("url".to_string(), json!("/api/v1/fleet/status"));
        let res = tool.execute(HashMap::new(), &config, &ctx()).await;
        assert!(res.is_err());
    }

    #[test]
    fn registers_the_tool() {
        let mut reg = ToolRegistry::new();
        register_net_tools(&mut reg);
        assert!(reg.get("net/http_request").is_some());
    }

    // A real POST round-trip against a tiny in-process axum server proves the
    // node actually writes to an HTTP API (no mocks).
    #[tokio::test]
    async fn posts_json_body_and_flattens_response() {
        use axum::routing::post;
        use axum::{Json, Router};

        async fn echo(Json(v): Json<Value>) -> Json<Value> {
            // Echo back an id derived from the body so we can assert flattening.
            Json(json!({"id": v["who"], "received": v}))
        }
        let app = Router::new().route("/ingest", post(echo));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let tool = HttpRequestTool;
        let mut config = HashMap::new();
        config.insert("url".to_string(), json!(format!("http://{addr}/ingest")));
        config.insert("method".to_string(), json!("POST"));
        config.insert("body".to_string(), json!({"who": "${name}"}));
        let mut inputs = HashMap::new();
        inputs.insert("name".to_string(), json!("centro"));

        let out = tool.execute(inputs, &config, &ctx()).await.unwrap();
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["http_status"], json!(200));
        // Response object flattened: `id` is now a top-level output field.
        assert_eq!(out["id"], json!("centro"));
        assert_eq!(out["response"]["received"]["who"], json!("centro"));
    }

    #[tokio::test]
    async fn non_2xx_fails_unless_ignored() {
        use axum::http::StatusCode;
        use axum::routing::post;
        use axum::Router;

        async fn boom() -> StatusCode {
            StatusCode::BAD_REQUEST
        }
        let app = Router::new().route("/boom", post(boom));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let tool = HttpRequestTool;
        let mut config = HashMap::new();
        config.insert("url".to_string(), json!(format!("http://{addr}/boom")));
        // Default: 400 → node error.
        assert!(tool.execute(HashMap::new(), &config, &ctx()).await.is_err());
        // ignore_http_errors → ok=false, no error.
        config.insert("ignore_http_errors".to_string(), json!(true));
        let out = tool.execute(HashMap::new(), &config, &ctx()).await.unwrap();
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["http_status"], json!(400));
    }
}
