//! CSS theme definitions for rendered HTML. REGLA-371: CSS-only, no JavaScript.

/// A named CSS theme for rendered HTML documents.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub css: String,
}

/// Dark theme -- slate/indigo palette on a dark background.
pub fn dark_theme() -> Theme {
    Theme {
        name: "dark".to_string(),
        css: r#"
body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    max-width: 800px; margin: 0 auto; padding: 2rem;
    line-height: 1.6; color: #e2e8f0; background: #0f172a;
}
h1, h2, h3, h4, h5, h6 { margin-top: 1.5em; margin-bottom: 0.5em; font-weight: 600; color: #f1f5f9; }
h1 { font-size: 1.8em; border-bottom: 2px solid #334155; padding-bottom: 0.3em; }
h2 { font-size: 1.4em; border-bottom: 1px solid #334155; padding-bottom: 0.2em; }
h3 { font-size: 1.2em; }
p { margin: 0.8em 0; }
a { color: #60a5fa; text-decoration: none; }
a:hover { text-decoration: underline; }
table { border-collapse: collapse; width: 100%; margin: 1em 0; }
th, td { border: 1px solid #334155; padding: 8px 12px; text-align: left; }
th { background: #1e293b; font-weight: 600; }
tr:nth-child(even) { background: #1e293b; }
code { background: #1e293b; padding: 2px 6px; border-radius: 4px; font-size: 0.9em; font-family: 'SF Mono', Monaco, monospace; }
pre { background: #020617; color: #e2e8f0; padding: 1rem; border-radius: 8px; overflow-x: auto; margin: 1em 0; border: 1px solid #1e293b; }
pre code { background: none; padding: 0; color: inherit; }
blockquote { border-left: 4px solid #818cf8; margin: 1em 0; padding: 0.5em 1em; background: #1e1b4b; color: #a5b4fc; }
ul, ol { padding-left: 1.5em; }
li { margin: 0.3em 0; }
hr { border: none; border-top: 1px solid #334155; margin: 2em 0; }
img { max-width: 100%; border-radius: 8px; }
.wiki-link { color: #818cf8; text-decoration: underline dotted; }
.wiki-link-broken { color: #f87171; text-decoration: line-through; }
"#
        .to_string(),
    }
}

/// Light theme -- clean grayscale on white background.
pub fn light_theme() -> Theme {
    Theme {
        name: "light".to_string(),
        css: r#"
body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    max-width: 800px; margin: 0 auto; padding: 2rem;
    line-height: 1.6; color: #1a1a1a; background: #ffffff;
}
h1, h2, h3, h4, h5, h6 { margin-top: 1.5em; margin-bottom: 0.5em; font-weight: 600; }
h1 { font-size: 1.8em; border-bottom: 2px solid #e5e7eb; padding-bottom: 0.3em; }
h2 { font-size: 1.4em; border-bottom: 1px solid #e5e7eb; padding-bottom: 0.2em; }
h3 { font-size: 1.2em; }
p { margin: 0.8em 0; }
a { color: #2563eb; text-decoration: none; }
a:hover { text-decoration: underline; }
table { border-collapse: collapse; width: 100%; margin: 1em 0; }
th, td { border: 1px solid #d1d5db; padding: 8px 12px; text-align: left; }
th { background: #f3f4f6; font-weight: 600; }
tr:nth-child(even) { background: #f9fafb; }
code { background: #f3f4f6; padding: 2px 6px; border-radius: 4px; font-size: 0.9em; font-family: 'SF Mono', Monaco, monospace; }
pre { background: #1e293b; color: #e2e8f0; padding: 1rem; border-radius: 8px; overflow-x: auto; margin: 1em 0; }
pre code { background: none; padding: 0; color: inherit; }
blockquote { border-left: 4px solid #6366f1; margin: 1em 0; padding: 0.5em 1em; background: #f5f3ff; color: #4338ca; }
ul, ol { padding-left: 1.5em; }
li { margin: 0.3em 0; }
hr { border: none; border-top: 1px solid #e5e7eb; margin: 2em 0; }
img { max-width: 100%; border-radius: 8px; }
.wiki-link { color: #6366f1; text-decoration: underline dotted; }
.wiki-link-broken { color: #ef4444; text-decoration: line-through; }
"#
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dark_theme() {
        let t = dark_theme();
        assert_eq!(t.name, "dark");
        assert!(t.css.contains("background: #0f172a"));
        assert!(t.css.contains(".wiki-link"));
    }

    #[test]
    fn test_light_theme() {
        let t = light_theme();
        assert_eq!(t.name, "light");
        assert!(t.css.contains("background: #ffffff"));
        assert!(t.css.contains(".wiki-link"));
    }
}
