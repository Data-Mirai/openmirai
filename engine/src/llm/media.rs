//! Media file handling for multimodal LLM requests.
//!
//! Reads local files, detects MIME types, validates against provider
//! capabilities, and encodes to base64 for inline delivery to LLM APIs.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A media attachment for multimodal LLM messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaContent {
    /// MIME type (e.g. `"audio/mp4"`, `"image/png"`).
    pub mime_type: String,
    /// Base64-encoded file content. Empty when the file travels by reference
    /// (`file_uri` set, or `pending_upload` awaiting upload).
    pub data: String,
    /// Original file path (for logging/debug — and the upload source when
    /// `pending_upload` is true).
    pub source_path: Option<String>,
    /// Remote file reference (Gemini Files API `file.uri`) once uploaded.
    /// PRD-018: large media travels by reference, not inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_uri: Option<String>,
    /// True when the file exceeds the inline limit and must be uploaded
    /// (Files API) by the adapter before generating. PRD-018.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending_upload: bool,
}

// ---------------------------------------------------------------------------
// MIME detection
// ---------------------------------------------------------------------------

/// Static map: file extension → MIME type.
fn extension_to_mime() -> &'static HashMap<&'static str, &'static str> {
    use std::sync::OnceLock;
    static MAP: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        // Audio
        m.insert("m4a", "audio/mp4");
        m.insert("mp3", "audio/mpeg");
        m.insert("wav", "audio/wav");
        m.insert("ogg", "audio/ogg");
        m.insert("flac", "audio/flac");
        // Video
        m.insert("mov", "video/quicktime");
        m.insert("mp4", "video/mp4");
        m.insert("webm", "video/webm");
        // Image
        m.insert("png", "image/png");
        m.insert("jpg", "image/jpeg");
        m.insert("jpeg", "image/jpeg");
        m.insert("webp", "image/webp");
        m.insert("gif", "image/gif");
        m
    })
}

// ---------------------------------------------------------------------------
// Provider support
// ---------------------------------------------------------------------------

const IMAGES_ONLY: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];
const GEMINI_ALL: &[&str] = &[
    "audio/mp4",
    "audio/mpeg",
    "audio/wav",
    "audio/ogg",
    "audio/flac",
    "video/quicktime",
    "video/mp4",
    "video/webm",
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
];

/// Returns the MIME types supported by a given provider (zero allocation).
pub fn supported_mimes_for_provider(provider: &str) -> &'static [&'static str] {
    match provider {
        "gemini" => GEMINI_ALL,
        _ => IMAGES_ONLY,
    }
}

/// Max size for inline (base64-in-request) delivery: 20 MB — Gemini's
/// documented request limit. Larger files go by reference (Files API).
const INLINE_MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

/// Hard cap: Gemini Files API limit per file (2 GB). PRD-018.
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// Providers that support by-reference delivery for files over the inline limit.
fn provider_supports_file_upload(provider: &str) -> bool {
    provider == "gemini"
}

// ---------------------------------------------------------------------------
// Core function: read_media_file
// ---------------------------------------------------------------------------

/// Read a media file, validate it, and return base64-encoded content.
///
/// Validates: existence, non-empty, size ≤ 20MB, known extension, MIME
/// supported by provider.
pub fn read_media_file(file_path: &str, provider_name: &str) -> Result<MediaContent, String> {
    let path = Path::new(file_path);

    // 1. File must exist
    if !path.exists() {
        return Err(format!("media file not found: {file_path}"));
    }

    // 2. Get metadata (size check before reading)
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("failed to read media file '{file_path}': {e}"))?;

    if metadata.len() == 0 {
        return Err(format!("media file is empty: {file_path}"));
    }

    if metadata.len() > MAX_FILE_SIZE {
        let size_gb = metadata.len() as f64 / (1024.0 * 1024.0 * 1024.0);
        return Err(format!(
            "media file too large: {size_gb:.1}GB (max 2GB — Files API limit)"
        ));
    }

    // PRD-018: files over the inline limit travel by reference (Files API).
    // Defer the upload to the adapter (it owns the API key and HTTP client);
    // don't read the bytes here — no 200MB base64 blobs in RAM.
    let needs_upload = metadata.len() > INLINE_MAX_FILE_SIZE;
    if needs_upload && !provider_supports_file_upload(provider_name) {
        let size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
        return Err(format!(
            "media file too large for inline delivery: {size_mb:.1}MB (max 20MB) — provider '{provider_name}' has no file upload support"
        ));
    }

    // 3. Detect MIME from extension
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    let ext_str = ext.as_deref().ok_or_else(|| {
        let supported: Vec<_> = extension_to_mime()
            .keys()
            .map(|k| format!(".{k}"))
            .collect();
        format!(
            "cannot detect media type: file has no extension. Supported: {}",
            supported.join(", ")
        )
    })?;

    let mime_map = extension_to_mime();
    let mime_type = mime_map.get(ext_str).ok_or_else(|| {
        let supported: Vec<_> = mime_map.keys().map(|k| format!(".{k}")).collect();
        format!(
            "unsupported file extension: '.{ext_str}'. Supported: {}",
            supported.join(", ")
        )
    })?;

    // 4. Check provider supports this MIME
    let supported = supported_mimes_for_provider(provider_name);
    if !supported.contains(mime_type) {
        let supported_list: Vec<_> = supported.to_vec();
        return Err(format!(
            "media type '{}' not supported by provider '{}'. Supported: {}",
            mime_type,
            provider_name,
            supported_list.join(", ")
        ));
    }

    // 5. Inline path: read and encode (scope bytes so they drop before
    //    MediaContent is built). Upload path: bytes stay on disk until the
    //    adapter streams them to the Files API.
    let b64 = if needs_upload {
        String::new()
    } else {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("failed to read media file '{file_path}': {e}"))?;
        STANDARD.encode(&bytes)
    };

    Ok(MediaContent {
        mime_type: mime_type.to_string(),
        data: b64,
        source_path: Some(file_path.to_string()),
        file_uri: None,
        pending_upload: needs_upload,
    })
}

// ---------------------------------------------------------------------------
// PRD-010: FileRef — file references as first-class graph data
// ---------------------------------------------------------------------------

/// The `_type` discriminator for FileRef JSON objects.
pub const FILE_REF_TYPE: &str = "file_ref";

/// Key used to pass media through the LLMResource context array.
/// The adapter bridge extracts this and attaches it to the user prompt message.
pub const USER_MEDIA_KEY: &str = "__user_media";

/// Build a context entry that carries media to the adapter bridge.
pub fn user_media_entry(media: &MediaContent) -> serde_json::Value {
    let media_json = serde_json::to_value([media]).unwrap_or(serde_json::json!([]));
    serde_json::json!({ USER_MEDIA_KEY: media_json })
}

/// Detect MIME type from file extension. Returns `application/octet-stream` for unknown.
pub fn mime_from_extension(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some(e) => extension_to_mime()
            .get(e)
            .copied()
            .unwrap_or("application/octet-stream"),
        None => "application/octet-stream",
    }
}

/// Create a FileRef JSON value from a file path.
///
/// Returns `None` if the file does not exist. The path is resolved to absolute
/// using `base_dir` if provided and the path is relative.
pub fn create_file_ref(file_path: &str, base_dir: Option<&str>) -> Option<serde_json::Value> {
    let path = Path::new(file_path);
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(base) = base_dir {
        Path::new(base).join(path)
    } else {
        // Try to canonicalize, fall back to as-is.
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    if !abs_path.exists() {
        return None;
    }

    let metadata = std::fs::metadata(&abs_path).ok()?;
    let mime = mime_from_extension(&abs_path);
    let abs_str = abs_path.to_string_lossy().to_string();

    Some(serde_json::json!({
        "_type": FILE_REF_TYPE,
        "path": abs_str,
        "mime_type": mime,
        "size_bytes": metadata.len(),
    }))
}

/// Check if a `serde_json::Value` is a valid FileRef object.
pub fn is_file_ref(value: &serde_json::Value) -> bool {
    value.get("_type").and_then(|v| v.as_str()) == Some(FILE_REF_TYPE)
        && value.get("path").and_then(|v| v.as_str()).is_some()
}

/// Resolve a tool input that may be a string path or a FileRef object.
///
/// - `Value::String` → returns the string as-is (backward compat).
/// - `Value::Object` with `_type: "file_ref"` → extracts and returns `path`.
/// - Other → stringifies the value.
pub fn resolve_file_input(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(_) if is_file_ref(value) => value
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn read_media_file_not_found() {
        let result = read_media_file("/nonexistent/file.png", "gemini");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("media file not found"));
    }

    #[test]
    fn read_media_file_empty() {
        let tmp = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = read_media_file(path, "gemini");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("media file is empty"));
    }

    #[test]
    fn read_media_file_unsupported_extension() {
        let mut tmp = tempfile::Builder::new().suffix(".xyz").tempfile().unwrap();
        tmp.write_all(b"data").unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = read_media_file(path, "gemini");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported file extension"));
    }

    #[test]
    fn read_media_file_provider_unsupported_mime() {
        let mut tmp = tempfile::Builder::new().suffix(".m4a").tempfile().unwrap();
        tmp.write_all(b"fake audio data").unwrap();
        let path = tmp.path().to_str().unwrap();
        // Claude doesn't support audio
        let result = read_media_file(path, "claude");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("not supported by provider 'claude'"));
    }

    #[test]
    fn read_media_file_happy_path_png() {
        let mut tmp = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        tmp.write_all(b"\x89PNG\r\n\x1a\n fake png").unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = read_media_file(path, "gemini").unwrap();
        assert_eq!(result.mime_type, "image/png");
        assert!(!result.data.is_empty());
        assert_eq!(result.source_path.as_deref(), Some(path));
    }

    #[test]
    fn read_media_file_happy_path_m4a_gemini() {
        let mut tmp = tempfile::Builder::new().suffix(".m4a").tempfile().unwrap();
        tmp.write_all(b"fake m4a audio").unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = read_media_file(path, "gemini").unwrap();
        assert_eq!(result.mime_type, "audio/mp4");
    }

    #[test]
    fn read_media_file_happy_path_mp4_video_gemini() {
        let mut tmp = tempfile::Builder::new().suffix(".mp4").tempfile().unwrap();
        tmp.write_all(b"fake mp4 video").unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = read_media_file(path, "gemini").unwrap();
        assert_eq!(result.mime_type, "video/mp4");
    }

    #[test]
    fn read_media_file_no_extension() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"data").unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = read_media_file(path, "gemini");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no extension"));
    }

    #[test]
    fn supported_mimes_gemini_has_audio() {
        let mimes = supported_mimes_for_provider("gemini");
        assert!(mimes.contains(&"audio/mp4"));
        assert!(mimes.contains(&"video/mp4"));
        assert!(mimes.contains(&"image/png"));
    }

    #[test]
    fn supported_mimes_claude_no_audio() {
        let mimes = supported_mimes_for_provider("claude");
        assert!(!mimes.contains(&"audio/mp4"));
        assert!(!mimes.contains(&"video/mp4"));
        assert!(mimes.contains(&"image/png"));
    }

    #[test]
    fn base64_roundtrip() {
        let original = b"Hello, world!";
        let encoded = STANDARD.encode(original);
        let decoded = STANDARD.decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    // --- PRD-018: large media travels by reference ---

    #[test]
    fn read_media_file_small_stays_inline() {
        let mut tmp = tempfile::Builder::new().suffix(".m4a").tempfile().unwrap();
        tmp.write_all(b"small audio").unwrap();
        let result = read_media_file(tmp.path().to_str().unwrap(), "gemini").unwrap();
        assert!(!result.pending_upload);
        assert!(result.file_uri.is_none());
        assert!(!result.data.is_empty());
    }

    #[test]
    fn read_media_file_over_20mb_defers_upload_no_bytes_in_ram() {
        let mut tmp = tempfile::Builder::new().suffix(".m4a").tempfile().unwrap();
        // 21 MB of zeros — over the inline limit, way under the 2GB cap.
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..21 {
            tmp.write_all(&chunk).unwrap();
        }
        tmp.flush().unwrap();
        let result = read_media_file(tmp.path().to_str().unwrap(), "gemini").unwrap();
        assert!(result.pending_upload);
        assert!(result.file_uri.is_none());
        assert!(result.data.is_empty(), "no base64 blob for upload path");
        assert!(result.source_path.is_some());
        assert_eq!(result.mime_type, "audio/mp4");
    }

    #[test]
    fn read_media_file_over_20mb_rejected_for_provider_without_upload() {
        let mut tmp = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..21 {
            tmp.write_all(&chunk).unwrap();
        }
        tmp.flush().unwrap();
        let result = read_media_file(tmp.path().to_str().unwrap(), "claude");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no file upload support"));
    }

    #[test]
    fn media_content_serde_roundtrip_with_upload_fields() {
        // The carrier (__user_media) serializes MediaContent across the
        // bridge — upload fields must survive the round trip.
        let mc = MediaContent {
            mime_type: "audio/mp4".into(),
            data: String::new(),
            source_path: Some("/tmp/big.m4a".into()),
            file_uri: None,
            pending_upload: true,
        };
        let json = serde_json::to_value(&mc).unwrap();
        let back: MediaContent = serde_json::from_value(json).unwrap();
        assert!(back.pending_upload);
        assert!(back.file_uri.is_none());
        assert_eq!(back.source_path.as_deref(), Some("/tmp/big.m4a"));
    }

    #[test]
    fn media_content_deserializes_legacy_shape_without_upload_fields() {
        // Old serialized MediaContent (pre PRD-018) must still deserialize.
        let legacy = serde_json::json!({
            "mime_type": "audio/ogg",
            "data": "dGVzdA==",
            "source_path": "/tmp/a.ogg"
        });
        let mc: MediaContent = serde_json::from_value(legacy).unwrap();
        assert!(!mc.pending_upload);
        assert!(mc.file_uri.is_none());
    }

    // --- PRD-010: FileRef tests ---

    #[test]
    fn create_file_ref_happy_path() {
        let mut tmp = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        tmp.write_all(b"fake png data 1234").unwrap();
        let path = tmp.path().to_str().unwrap();
        let file_ref = create_file_ref(path, None).unwrap();
        assert_eq!(file_ref["_type"], "file_ref");
        assert_eq!(file_ref["mime_type"], "image/png");
        assert_eq!(file_ref["size_bytes"], 18);
        assert!(file_ref["path"].as_str().unwrap().contains(".png"));
    }

    #[test]
    fn create_file_ref_nonexistent_returns_none() {
        let result = create_file_ref("/nonexistent/file.png", None);
        assert!(result.is_none());
    }

    #[test]
    fn create_file_ref_relative_with_base_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.jpg");
        std::fs::write(&file_path, b"jpeg data").unwrap();
        let file_ref = create_file_ref("test.jpg", Some(dir.path().to_str().unwrap())).unwrap();
        assert_eq!(file_ref["_type"], "file_ref");
        assert_eq!(file_ref["mime_type"], "image/jpeg");
        assert!(file_ref["path"].as_str().unwrap().ends_with("test.jpg"));
    }

    #[test]
    fn create_file_ref_no_extension_uses_octet_stream() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("data_file");
        std::fs::write(&file_path, b"binary data").unwrap();
        let file_ref = create_file_ref(file_path.to_str().unwrap(), None).unwrap();
        assert_eq!(file_ref["mime_type"], "application/octet-stream");
    }

    #[test]
    fn is_file_ref_valid() {
        let valid = serde_json::json!({
            "_type": "file_ref",
            "path": "/tmp/test.png",
            "mime_type": "image/png",
            "size_bytes": 100
        });
        assert!(is_file_ref(&valid));
    }

    #[test]
    fn is_file_ref_invalid_no_type() {
        let invalid = serde_json::json!({"path": "/tmp/test.png"});
        assert!(!is_file_ref(&invalid));
    }

    #[test]
    fn is_file_ref_invalid_wrong_type() {
        let invalid = serde_json::json!({"_type": "something_else", "path": "/tmp/test.png"});
        assert!(!is_file_ref(&invalid));
    }

    #[test]
    fn is_file_ref_string_is_not_file_ref() {
        assert!(!is_file_ref(&serde_json::json!("/tmp/test.png")));
    }

    #[test]
    fn resolve_file_input_string_path() {
        let val = serde_json::json!("/tmp/test.png");
        assert_eq!(resolve_file_input(&val), "/tmp/test.png");
    }

    #[test]
    fn resolve_file_input_file_ref_extracts_path() {
        let val = serde_json::json!({
            "_type": "file_ref",
            "path": "/tmp/resolved.png",
            "mime_type": "image/png",
            "size_bytes": 100
        });
        assert_eq!(resolve_file_input(&val), "/tmp/resolved.png");
    }

    #[test]
    fn resolve_file_input_null_returns_empty() {
        assert_eq!(resolve_file_input(&serde_json::json!(null)), "");
    }

    #[test]
    fn resolve_file_input_number_stringifies() {
        assert_eq!(resolve_file_input(&serde_json::json!(42)), "42");
    }
}
