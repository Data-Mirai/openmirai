"""CSS themes for the Render Engine. REGLA-371: CSS-only, no JavaScript."""

THEME_DEFAULT = """
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
.toc { background: #f8fafc; padding: 1rem 1.5rem; border-radius: 8px; margin-bottom: 2rem; border: 1px solid #e2e8f0; }
.toc h2 { font-size: 1em; margin-top: 0; border: none; }
.toc ul { list-style: none; padding-left: 0; }
.toc li { margin: 0.2em 0; }
.toc a { color: #4b5563; }
.vault-link { color: #6366f1; text-decoration: underline dotted; }
.vault-link-broken { color: #ef4444; text-decoration: line-through; }
.metadata { color: #6b7280; font-size: 0.85rem; border-bottom: 1px solid #e5e7eb; padding-bottom: 1rem; margin-bottom: 2rem; }
.metadata span { margin-right: 1.5em; }
"""

THEME_DARK = """
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
.toc { background: #1e293b; padding: 1rem 1.5rem; border-radius: 8px; margin-bottom: 2rem; border: 1px solid #334155; }
.toc h2 { font-size: 1em; margin-top: 0; border: none; }
.toc ul { list-style: none; padding-left: 0; }
.toc a { color: #94a3b8; }
.vault-link { color: #818cf8; text-decoration: underline dotted; }
.vault-link-broken { color: #f87171; text-decoration: line-through; }
.metadata { color: #94a3b8; font-size: 0.85rem; border-bottom: 1px solid #334155; padding-bottom: 1rem; margin-bottom: 2rem; }
.metadata span { margin-right: 1.5em; }
"""

THEME_MINIMAL = """
body {
    font-family: Georgia, 'Times New Roman', serif;
    max-width: 680px; margin: 0 auto; padding: 2rem;
    line-height: 1.8; color: #333; background: #fefefe;
}
h1, h2, h3 { font-weight: normal; }
h1 { font-size: 1.6em; }
h2 { font-size: 1.3em; }
h3 { font-size: 1.1em; }
p { margin: 1em 0; }
a { color: #333; text-decoration: underline; }
table { border-collapse: collapse; width: 100%; margin: 1em 0; }
th, td { border-bottom: 1px solid #ddd; padding: 6px 10px; text-align: left; }
th { font-weight: 600; }
code { font-family: monospace; font-size: 0.9em; }
pre { padding: 1rem; overflow-x: auto; background: #f5f5f5; }
blockquote { border-left: 2px solid #999; margin: 1em 0; padding: 0.5em 1em; color: #666; }
hr { border: none; border-top: 1px solid #ccc; margin: 2em 0; }
.toc { margin-bottom: 2rem; }
.toc ul { list-style: none; padding-left: 0; }
.vault-link { color: #555; text-decoration: underline dotted; }
.vault-link-broken { color: #c00; text-decoration: line-through; }
.metadata { color: #999; font-size: 0.85rem; margin-bottom: 2rem; }
"""

THEME_REPORT = """
body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    max-width: 900px; margin: 0 auto; padding: 2rem 3rem;
    line-height: 1.6; color: #1a1a1a; background: #ffffff;
}
h1 { font-size: 2em; color: #111827; border-bottom: 3px solid #6366f1; padding-bottom: 0.3em; }
h2 { font-size: 1.5em; color: #1f2937; margin-top: 2em; }
h3 { font-size: 1.2em; color: #374151; }
p { margin: 0.8em 0; }
a { color: #4f46e5; }
table { border-collapse: collapse; width: 100%; margin: 1.5em 0; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
th, td { border: 1px solid #d1d5db; padding: 10px 14px; }
th { background: #6366f1; color: white; font-weight: 600; }
tr:nth-child(even) { background: #f9fafb; }
code { background: #f3f4f6; padding: 2px 6px; border-radius: 4px; font-size: 0.9em; }
pre { background: #1e293b; color: #e2e8f0; padding: 1.2rem; border-radius: 8px; overflow-x: auto; }
pre code { background: none; padding: 0; color: inherit; }
blockquote { border-left: 4px solid #6366f1; background: #eef2ff; padding: 1em; margin: 1em 0; border-radius: 0 8px 8px 0; }
hr { border: none; border-top: 2px solid #e5e7eb; margin: 2em 0; }
.toc { background: #f8fafc; padding: 1.5rem 2rem; border-radius: 12px; margin-bottom: 2rem; border: 1px solid #e2e8f0; }
.toc h2 { font-size: 1em; margin-top: 0; color: #6366f1; }
.vault-link { color: #6366f1; text-decoration: underline dotted; }
.vault-link-broken { color: #ef4444; text-decoration: line-through; }
.metadata { background: #f8fafc; padding: 1rem; border-radius: 8px; color: #6b7280; font-size: 0.85rem; margin-bottom: 2rem; }
.metadata span { display: inline-block; margin-right: 2em; }
"""

_THEMES = {
    "default": THEME_DEFAULT,
    "dark": THEME_DARK,
    "minimal": THEME_MINIMAL,
    "report": THEME_REPORT,
}


def get_theme(name: str) -> str:
    """Get CSS for a theme by name. Falls back to default."""
    return _THEMES.get(name, THEME_DEFAULT)
