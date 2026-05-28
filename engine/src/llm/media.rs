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
    /// Base64-encoded file content.
    pub data: String,
    /// Original file path (for logging/debug).
    pub source_path: Option<String>,
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

/// Returns the set of MIME types supported by a given provider.
pub fn supported_mimes_for_provider(provider: &str) -> Vec<&'static str> {
    match provider {
        "gemini" => vec![
            // Audio
            "audio/mp4", "audio/mpeg", "audio/wav", "audio/ogg", "audio/flac",
            // Video
            "video/quicktime", "video/mp4", "video/webm",
            // Image
            "image/png", "image/jpeg", "image/webp", "image/gif",
        ],
        "claude" => vec![
            "image/png", "image/jpeg", "image/webp", "image/gif",
        ],
        "openai" | "groq" | "openrouter" | "nvidia_nim" => vec![
            "image/png", "image/jpeg", "image/webp", "image/gif",
        ],
        "ollama" => vec![
            "image/png", "image/jpeg", "image/webp", "image/gif",
        ],
        _ => vec![
            // Unknown provider: allow images only (safest default).
            "image/png", "image/jpeg", "image/webp", "image/gif",
        ],
    }
}

/// Max file size in bytes (20 MB).
const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

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
        let size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
        return Err(format!(
            "media file too large: {size_mb:.1}MB (max 20MB)"
        ));
    }

    // 3. Detect MIME from extension
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    let ext_str = ext.as_deref().ok_or_else(|| {
        let supported: Vec<_> = extension_to_mime().keys().map(|k| format!(".{k}")).collect();
        format!(
            "cannot detect media type: file has no extension. Supported: {}",
            supported.join(", ")
        )
    })?;

    let mime_map = extension_to_mime();
    let mime_type = mime_map.get(&*ext_str).ok_or_else(|| {
        let supported: Vec<_> = mime_map.keys().map(|k| format!(".{k}")).collect();
        format!(
            "unsupported file extension: '.{ext_str}'. Supported: {}",
            supported.join(", ")
        )
    })?;

    // 4. Check provider supports this MIME
    let supported = supported_mimes_for_provider(provider_name);
    if !supported.contains(mime_type) {
        let supported_list: Vec<_> = supported.iter().copied().collect();
        return Err(format!(
            "media type '{}' not supported by provider '{}'. Supported: {}",
            mime_type, provider_name, supported_list.join(", ")
        ));
    }

    // 5. Read and encode
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read media file '{file_path}': {e}"))?;

    let b64 = STANDARD.encode(&bytes);

    Ok(MediaContent {
        mime_type: mime_type.to_string(),
        data: b64,
        source_path: Some(file_path.to_string()),
    })
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
        assert!(result.unwrap_err().contains("not supported by provider 'claude'"));
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
}
