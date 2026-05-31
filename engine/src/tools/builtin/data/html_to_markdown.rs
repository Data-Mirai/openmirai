//! HtmlToMarkdownTool

use super::*;

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
pub(crate) fn convert_html_to_md(html: &str) -> String {
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
        let html = inputs.get("html").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::ExecutionFailed {
                tool_type: "data/html_to_markdown".into(),
                message: "input 'html' is required".into(),
            }
        })?;

        let markdown = convert_html_to_md(html);
        let length = markdown.len();

        let mut out = HashMap::new();
        out.insert("markdown".to_string(), json!(markdown));
        out.insert("length".to_string(), json!(length));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Registration helper
