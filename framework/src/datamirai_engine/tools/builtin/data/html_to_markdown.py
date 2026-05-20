"""HTML to Markdown converter — custom implementation using stdlib only.

Converts HTML to clean Markdown preserving semantic structure.
Includes Readability-style content extraction to find main content.
REGLA-354: Never includes CSS, JavaScript, or HTML attributes.
"""

from __future__ import annotations

import re
from html.parser import HTMLParser
from typing import Any


def html_to_markdown(html: str, *, extract_main: bool = True) -> str:
    """Convert HTML to Markdown.

    If extract_main=True, first extracts the main content area
    (article, main, or highest text-density block) before converting.
    """
    if extract_main:
        html = extract_main_content(html)

    converter = _HtmlToMarkdown()
    converter.feed(html)
    md = converter.get_output()

    # Clean up excessive blank lines
    md = re.sub(r"\n{3,}", "\n\n", md)
    return md.strip()


def extract_main_content(html: str) -> str:
    """Extract the main content area from HTML (Readability-style).

    Priority: <article> > <main> > [role="main"] > highest text density block.
    """
    # Try semantic containers first
    for tag in ("article", "main"):
        match = re.search(
            rf"<{tag}[^>]*>(.*?)</{tag}>",
            html,
            re.DOTALL | re.IGNORECASE,
        )
        if match:
            return match.group(1)

    # Try role="main"
    match = re.search(
        r'<\w+[^>]*role\s*=\s*["\']main["\'][^>]*>(.*?)</\w+>',
        html,
        re.DOTALL | re.IGNORECASE,
    )
    if match:
        return match.group(1)

    # Try <div class="content"> or similar common patterns
    for cls in ("content", "post-content", "entry-content", "article-body", "post-body"):
        match = re.search(
            rf'<div[^>]*class\s*=\s*["\'][^"\']*{cls}[^"\']*["\'][^>]*>(.*?)</div>',
            html,
            re.DOTALL | re.IGNORECASE,
        )
        if match:
            return match.group(1)

    # Fallback: strip head, scripts, styles, nav, footer and return body
    cleaned = html
    for pattern in [
        r"<head[^>]*>.*?</head>",
        r"<script[^>]*>.*?</script>",
        r"<style[^>]*>.*?</style>",
        r"<noscript[^>]*>.*?</noscript>",
        r"<nav[^>]*>.*?</nav>",
        r"<footer[^>]*>.*?</footer>",
        r"<header[^>]*>.*?</header>",
        r"<aside[^>]*>.*?</aside>",
    ]:
        cleaned = re.sub(pattern, "", cleaned, flags=re.DOTALL | re.IGNORECASE)

    # Extract body if present
    body_match = re.search(r"<body[^>]*>(.*?)</body>", cleaned, re.DOTALL | re.IGNORECASE)
    if body_match:
        return body_match.group(1)

    return cleaned


# Skip tags — content inside these is discarded
_SKIP_TAGS = frozenset({"script", "style", "noscript", "svg", "iframe"})

# Block tags that produce newlines
_BLOCK_TAGS = frozenset({
    "p", "div", "section", "article", "main", "aside",
    "header", "footer", "nav", "figure", "figcaption",
    "blockquote", "details", "summary",
})


class _HtmlToMarkdown(HTMLParser):
    """Custom HTML parser that produces Markdown output."""

    def __init__(self) -> None:
        super().__init__()
        self._output: list[str] = []
        self._skip_depth = 0
        self._tag_stack: list[str] = []

        # List state
        self._list_stack: list[str] = []  # "ul" or "ol"
        self._ol_counters: list[int] = []

        # Table state
        self._in_table = False
        self._table_row: list[str] = []
        self._table_rows: list[list[str]] = []
        self._is_header_row = False
        self._in_cell = False
        self._cell_content: list[str] = []

        # Code block state
        self._in_pre = False
        self._pre_content: list[str] = []

        # Inline code
        self._in_code = False

        # Link state
        self._in_link = False
        self._link_href = ""
        self._link_text: list[str] = []

        # Heading state
        self._in_heading = 0  # 0=not in heading, 1-6=heading level
        self._heading_text: list[str] = []

        # Emphasis
        self._in_strong = False
        self._in_em = False

        # Blockquote
        self._in_blockquote = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        tag = tag.lower()
        attrs_dict = dict(attrs)

        if tag in _SKIP_TAGS:
            self._skip_depth += 1
            return
        if self._skip_depth > 0:
            return

        self._tag_stack.append(tag)

        # Headings
        if tag in ("h1", "h2", "h3", "h4", "h5", "h6"):
            self._in_heading = int(tag[1])
            self._heading_text = []
            return

        # Links
        if tag == "a":
            self._in_link = True
            self._link_href = attrs_dict.get("href", "")
            self._link_text = []
            return

        # Images
        if tag == "img":
            alt = attrs_dict.get("alt", "")
            src = attrs_dict.get("src", "")
            if src:
                self._output.append(f"![{alt}]({src})")
            return

        # Emphasis
        if tag in ("strong", "b"):
            self._in_strong = True
            self._output.append("**")
            return
        if tag in ("em", "i"):
            self._in_em = True
            self._output.append("*")
            return

        # Code
        if tag == "code" and not self._in_pre:
            self._in_code = True
            self._output.append("`")
            return

        # Pre (code blocks)
        if tag == "pre":
            self._in_pre = True
            self._pre_content = []
            self._output.append("\n```\n")
            return

        # Lists
        if tag in ("ul", "ol"):
            self._list_stack.append(tag)
            if tag == "ol":
                self._ol_counters.append(0)
            return
        if tag == "li":
            indent = "  " * max(0, len(self._list_stack) - 1)
            if self._list_stack and self._list_stack[-1] == "ol":
                self._ol_counters[-1] += 1
                self._output.append(f"\n{indent}{self._ol_counters[-1]}. ")
            else:
                self._output.append(f"\n{indent}- ")
            return

        # Tables
        if tag == "table":
            self._in_table = True
            self._table_rows = []
            return
        if tag == "thead":
            self._is_header_row = True
            return
        if tag == "tr":
            self._table_row = []
            return
        if tag in ("th", "td"):
            self._in_cell = True
            self._cell_content = []
            if tag == "th":
                self._is_header_row = True
            return

        # Blockquote
        if tag == "blockquote":
            self._in_blockquote = True
            self._output.append("\n> ")
            return

        # Horizontal rule
        if tag == "hr":
            self._output.append("\n\n---\n\n")
            return

        # Line break
        if tag == "br":
            self._output.append("\n")
            return

        # Block elements — add spacing
        if tag in _BLOCK_TAGS:
            self._output.append("\n\n")

    def handle_endtag(self, tag: str) -> None:
        tag = tag.lower()

        if tag in _SKIP_TAGS:
            self._skip_depth = max(0, self._skip_depth - 1)
            return
        if self._skip_depth > 0:
            return

        if self._tag_stack and self._tag_stack[-1] == tag:
            self._tag_stack.pop()

        # Headings
        if tag in ("h1", "h2", "h3", "h4", "h5", "h6") and self._in_heading:
            prefix = "#" * self._in_heading
            text = "".join(self._heading_text).strip()
            self._output.append(f"\n\n{prefix} {text}\n\n")
            self._in_heading = 0
            return

        # Links
        if tag == "a" and self._in_link:
            text = "".join(self._link_text).strip()
            if self._link_href and text:
                self._output.append(f"[{text}]({self._link_href})")
            elif text:
                self._output.append(text)
            self._in_link = False
            return

        # Emphasis
        if tag in ("strong", "b") and self._in_strong:
            self._output.append("**")
            self._in_strong = False
            return
        if tag in ("em", "i") and self._in_em:
            self._output.append("*")
            self._in_em = False
            return

        # Code
        if tag == "code" and self._in_code and not self._in_pre:
            self._output.append("`")
            self._in_code = False
            return

        # Pre
        if tag == "pre" and self._in_pre:
            self._output.append("\n```\n")
            self._in_pre = False
            return

        # Lists
        if tag in ("ul", "ol"):
            if self._list_stack:
                self._list_stack.pop()
            if tag == "ol" and self._ol_counters:
                self._ol_counters.pop()
            self._output.append("\n")
            return

        # Tables
        if tag in ("th", "td") and self._in_cell:
            self._table_row.append("".join(self._cell_content).strip())
            self._in_cell = False
            return
        if tag == "tr" and self._in_table:
            self._table_rows.append(list(self._table_row))
            self._table_row = []
            return
        if tag == "thead":
            self._is_header_row = False
            return
        if tag == "table" and self._in_table:
            self._flush_table()
            self._in_table = False
            return

        # Blockquote
        if tag == "blockquote" and self._in_blockquote:
            self._in_blockquote = False
            self._output.append("\n")
            return

        # Block elements
        if tag in _BLOCK_TAGS:
            self._output.append("\n\n")

    def handle_data(self, data: str) -> None:
        if self._skip_depth > 0:
            return

        # Heading text
        if self._in_heading:
            self._heading_text.append(data)
            return

        # Link text
        if self._in_link:
            self._link_text.append(data)
            return

        # Table cell
        if self._in_cell:
            self._cell_content.append(data)
            return

        # Pre block
        if self._in_pre:
            self._output.append(data)
            return

        # Blockquote — prefix lines with >
        if self._in_blockquote:
            lines = data.split("\n")
            self._output.append("\n> ".join(lines))
            return

        # Normal text
        self._output.append(data)

    def _flush_table(self) -> None:
        """Convert accumulated table rows to Markdown pipe table."""
        if not self._table_rows:
            return

        # Determine column count
        max_cols = max(len(row) for row in self._table_rows)
        if max_cols == 0:
            return

        self._output.append("\n\n")

        # First row as header
        header = self._table_rows[0]
        while len(header) < max_cols:
            header.append("")
        self._output.append("| " + " | ".join(header) + " |\n")
        self._output.append("| " + " | ".join(["---"] * max_cols) + " |\n")

        # Remaining rows
        for row in self._table_rows[1:]:
            while len(row) < max_cols:
                row.append("")
            self._output.append("| " + " | ".join(row) + " |\n")

        self._output.append("\n")

    def get_output(self) -> str:
        return "".join(self._output)
