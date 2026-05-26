//! RenderEngine -- Markdown to self-contained HTML converter.
//!
//! REGLA-367: Output is always self-contained (embedded CSS, zero external refs).
//! REGLA-372: Custom parser, no external Markdown dependencies.

use crate::render::themes::Theme;
use regex::Regex;
use std::sync::LazyLock;

// Pre-compiled regexes (allocated once).
static RE_HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(#{1,6})\s+(.+)$").unwrap());
static RE_HR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\*{3,}|-{3,}|_{3,})\s*$").unwrap());
static RE_UL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+").unwrap());
static RE_OL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+\.\s+").unwrap());
static RE_SLUG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").unwrap());
static RE_INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());
static RE_IMAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap());
static RE_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap());
static RE_WIKILINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|]+?)(?:\|([^\]]+))?\]\]").unwrap());
static RE_BOLD_STAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").unwrap());
static RE_BOLD_UNDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"__(.+?)__").unwrap());
static RE_ITALIC_STAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*(.+?)\*").unwrap());
// Note: Rust regex crate does not support lookbehinds. We use a capturing group
// approach instead: match a non-word char (or start) before _text_ and a non-word
// char (or end) after it, and rebuild the replacement preserving the boundary chars.
static RE_ITALIC_UNDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|(?P<pre>[^a-zA-Z0-9]))_(?P<inner>.+?)_(?:(?P<post>[^a-zA-Z0-9])|$)").unwrap());

/// Custom Markdown-to-HTML render engine with embedded CSS themes.
pub struct RenderEngine {
    theme: Theme,
}

impl RenderEngine {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Convert Markdown to a complete, self-contained HTML document.
    /// REGLA-367: zero external CSS/JS references.
    pub fn render_markdown(&self, markdown: &str) -> String {
        let body = self.parse_markdown(markdown);
        self.wrap_html(&body, "")
    }

    /// Render Markdown and append raw chart HTML snippets after the body.
    pub fn render_with_charts(&self, markdown: &str, charts: &[String]) -> String {
        let body = self.parse_markdown(markdown);
        let charts_html = charts.join("\n");
        self.wrap_html(&body, &charts_html)
    }

    // ---- Internal Markdown parser (state machine) ----

    fn parse_markdown(&self, md: &str) -> String {
        let lines: Vec<&str> = md.split('\n').collect();
        let mut output: Vec<String> = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let stripped = line.trim();

            // Fenced code blocks
            if stripped.starts_with("```") {
                let lang = stripped[3..].trim();
                let mut code_lines: Vec<&str> = Vec::new();
                i += 1;
                while i < lines.len() && !lines[i].trim().starts_with("```") {
                    code_lines.push(lines[i]);
                    i += 1;
                }
                i += 1; // skip closing ```
                let code_content = escape_html(&code_lines.join("\n"));
                let lang_attr = if !lang.is_empty() {
                    format!(" class=\"language-{}\"", escape_html(lang))
                } else {
                    String::new()
                };
                output.push(format!("<pre><code{}>{}</code></pre>", lang_attr, code_content));
                continue;
            }

            // Headings
            if let Some(caps) = RE_HEADING.captures(stripped) {
                let level = caps[1].len();
                let text = caps[2].trim();
                let inline = self.process_inline(text);
                let lowered = text.to_lowercase();
                let slug = RE_SLUG.replace_all(&lowered, "-");
                let slug = slug.trim_matches('-');
                output.push(format!("<h{level} id=\"{slug}\">{inline}</h{level}>"));
                i += 1;
                continue;
            }

            // Horizontal rule
            if RE_HR.is_match(stripped) {
                output.push("<hr>".to_string());
                i += 1;
                continue;
            }

            // Blockquote
            if stripped.starts_with('>') {
                let mut bq_lines: Vec<String> = Vec::new();
                while i < lines.len() && lines[i].trim().starts_with('>') {
                    let raw = lines[i];
                    let content = raw.trim().strip_prefix('>').unwrap_or(raw);
                    let content = content.strip_prefix(' ').unwrap_or(content);
                    bq_lines.push(content.to_string());
                    i += 1;
                }
                let inner = self.parse_markdown(&bq_lines.join("\n"));
                output.push(format!("<blockquote>{inner}</blockquote>"));
                continue;
            }

            // Unordered list
            if RE_UL.is_match(stripped) {
                let (list_html, new_i) = self.parse_list(&lines, i, false);
                output.push(list_html);
                i = new_i;
                continue;
            }

            // Ordered list
            if RE_OL.is_match(stripped) {
                let (list_html, new_i) = self.parse_list(&lines, i, true);
                output.push(list_html);
                i = new_i;
                continue;
            }

            // Empty line
            if stripped.is_empty() {
                i += 1;
                continue;
            }

            // Paragraph -- collect contiguous non-empty, non-block-start lines
            let mut para_lines: Vec<&str> = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() && !self.is_block_start(lines[i])
            {
                para_lines.push(lines[i]);
                i += 1;
            }
            if !para_lines.is_empty() {
                let text = self.process_inline(&para_lines.join(" "));
                output.push(format!("<p>{text}</p>"));
            } else {
                // Safety: always advance to prevent infinite loop
                i += 1;
            }
        }

        output.join("\n")
    }

    /// Check if a line starts a new block element.
    fn is_block_start(&self, line: &str) -> bool {
        let s = line.trim();
        s.starts_with('#')
            || s.starts_with("```")
            || s.starts_with('>')
            || RE_UL.is_match(s)
            || RE_OL.is_match(s)
            || RE_HR.is_match(s)
    }

    /// Parse a list block (ordered or unordered).
    fn parse_list(&self, lines: &[&str], start: usize, ordered: bool) -> (String, usize) {
        let tag = if ordered { "ol" } else { "ul" };
        let pattern = if ordered { &*RE_OL } else { &*RE_UL };
        let mut items: Vec<String> = Vec::new();
        let mut i = start;

        while i < lines.len() {
            let stripped = lines[i].trim();
            if !pattern.is_match(stripped) {
                break;
            }
            let content = pattern.replace(stripped, "");
            let content = content.trim();
            items.push(format!("<li>{}</li>", self.process_inline(content)));
            i += 1;
        }

        (format!("<{tag}>{}</{tag}>", items.join("")), i)
    }

    /// Process inline Markdown elements: bold, italic, code, links, images, wiki-links.
    fn process_inline(&self, text: &str) -> String {
        let text = escape_html(text);
        // Code spans first (protect content inside)
        let text = RE_INLINE_CODE.replace_all(&text, "<code>$1</code>");
        // Images before links (![alt](src) vs [text](url))
        let text = RE_IMAGE.replace_all(&text, "<img src=\"$2\" alt=\"$1\">");
        // Standard links
        let text = RE_LINK.replace_all(&text, "<a href=\"$2\">$1</a>");
        // Wiki-links
        let text = RE_WIKILINK.replace_all(&text, |caps: &regex::Captures| {
            let reference = caps.get(1).map_or("", |m| m.as_str()).trim();
            let display = caps
                .get(2)
                .map_or(reference, |m| m.as_str())
                .trim();
            format!(
                "<a class=\"wiki-link\" href=\"{ref_}\">{display}</a>",
                ref_ = reference,
                display = display,
            )
        });
        // Bold (**text** and __text__)
        let text = RE_BOLD_STAR.replace_all(&text, "<strong>$1</strong>");
        let text = RE_BOLD_UNDER.replace_all(&text, "<strong>$1</strong>");
        // Italic (*text* and _text_)
        let text = RE_ITALIC_STAR.replace_all(&text, "<em>$1</em>");
        let text = RE_ITALIC_UNDER.replace_all(&text, |caps: &regex::Captures| {
            let pre = caps.name("pre").map_or("", |m| m.as_str());
            let inner = caps.name("inner").map_or("", |m| m.as_str());
            let post = caps.name("post").map_or("", |m| m.as_str());
            format!("{pre}<em>{inner}</em>{post}")
        });
        text.to_string()
    }

    /// Wrap rendered body HTML in a complete self-contained HTML document.
    fn wrap_html(&self, body: &str, extra_body: &str) -> String {
        format!(
            r#"<!DOCTYPE html>
<html lang="es">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Report</title>
    <style>{css}</style>
</head>
<body>
    <main>{body}</main>
    {extra}
</body>
</html>"#,
            css = self.theme.css,
            body = body,
            extra = extra_body,
        )
    }
}

/// Escape HTML special characters.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::themes::light_theme;

    fn engine() -> RenderEngine {
        RenderEngine::new(light_theme())
    }

    #[test]
    fn test_heading() {
        let html = engine().render_markdown("# Hello World");
        assert!(html.contains("<h1"));
        assert!(html.contains("Hello World"));
        assert!(html.contains("</h1>"));
    }

    #[test]
    fn test_multiple_heading_levels() {
        let md = "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6";
        let html = engine().render_markdown(md);
        assert!(html.contains("<h1"));
        assert!(html.contains("<h2"));
        assert!(html.contains("<h3"));
        assert!(html.contains("<h4"));
        assert!(html.contains("<h5"));
        assert!(html.contains("<h6"));
    }

    #[test]
    fn test_bold() {
        let html = engine().render_markdown("This is **bold** text.");
        assert!(html.contains("<strong>bold</strong>"));
    }

    #[test]
    fn test_italic() {
        let html = engine().render_markdown("This is *italic* text.");
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn test_inline_code() {
        let html = engine().render_markdown("Use `cargo build` to compile.");
        assert!(html.contains("<code>cargo build</code>"));
    }

    #[test]
    fn test_fenced_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let html = engine().render_markdown(md);
        assert!(html.contains("<pre><code class=\"language-rust\">"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn test_link() {
        let html = engine().render_markdown("[Rust](https://rust-lang.org)");
        assert!(html.contains("<a href=\"https://rust-lang.org\">Rust</a>"));
    }

    #[test]
    fn test_image() {
        let html = engine().render_markdown("![alt text](image.png)");
        assert!(html.contains("<img src=\"image.png\" alt=\"alt text\">"));
    }

    #[test]
    fn test_unordered_list() {
        let md = "- One\n- Two\n- Three";
        let html = engine().render_markdown(md);
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>One</li>"));
        assert!(html.contains("<li>Two</li>"));
        assert!(html.contains("<li>Three</li>"));
        assert!(html.contains("</ul>"));
    }

    #[test]
    fn test_ordered_list() {
        let md = "1. First\n2. Second";
        let html = engine().render_markdown(md);
        assert!(html.contains("<ol>"));
        assert!(html.contains("<li>First</li>"));
        assert!(html.contains("<li>Second</li>"));
    }

    #[test]
    fn test_wiki_link() {
        let html = engine().render_markdown("See [[my-note]] for details.");
        assert!(html.contains("class=\"wiki-link\""));
        assert!(html.contains("href=\"my-note\""));
        assert!(html.contains(">my-note</a>"));
    }

    #[test]
    fn test_wiki_link_with_alias() {
        let html = engine().render_markdown("See [[my-note|My Note]] for details.");
        assert!(html.contains("href=\"my-note\""));
        assert!(html.contains(">My Note</a>"));
    }

    #[test]
    fn test_blockquote() {
        let md = "> This is quoted\n> Second line";
        let html = engine().render_markdown(md);
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("This is quoted"));
    }

    #[test]
    fn test_horizontal_rule() {
        let html = engine().render_markdown("---");
        assert!(html.contains("<hr>"));
    }

    #[test]
    fn test_self_contained_html() {
        let html = engine().render_markdown("Hello");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("</style>"));
        // No external references (REGLA-367)
        assert!(!html.contains("<link"));
        assert!(!html.contains("<script src="));
    }

    #[test]
    fn test_html_escaping() {
        let html = engine().render_markdown("Use <div> and & in text.");
        assert!(html.contains("&lt;div&gt;"));
        assert!(html.contains("&amp;"));
    }

    #[test]
    fn test_render_with_charts() {
        let chart = "<div>CHART_PLACEHOLDER</div>".to_string();
        let html = engine().render_with_charts("# Report", &[chart]);
        assert!(html.contains("CHART_PLACEHOLDER"));
        assert!(html.contains("<h1"));
    }

    #[test]
    fn test_paragraph() {
        let html = engine().render_markdown("A simple paragraph.");
        assert!(html.contains("<p>A simple paragraph.</p>"));
    }

    #[test]
    fn test_empty_input() {
        let html = engine().render_markdown("");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<main>"));
    }
}
