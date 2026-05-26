"""Render Engine — Custom Markdown to HTML converter.

REGLA-367: HTML is always self-contained (embedded CSS, zero external JS/CSS).
REGLA-370: Parser never executes JavaScript from markdown code blocks.
REGLA-372: Custom parser, no external dependencies.
"""

from __future__ import annotations

import html
import logging
import re
from typing import Any

from datamirai_engine.render.themes import get_theme

logger = logging.getLogger(__name__)

# Pre-compiled regexes (avoid recompiling on every call)
_RE_HEADING = re.compile(r"^\s*(#{1,6})\s+(.+)$")
_RE_HR = re.compile(r"^(\*{3,}|-{3,}|_{3,})\s*$")
_RE_UL = re.compile(r"^\s*[-*+]\s+")
_RE_OL = re.compile(r"^\s*\d+\.\s+")
_RE_TABLE_SEP = re.compile(r"^\s*\|[\s\-:|]+\|\s*$")
_RE_SLUG = re.compile(r"[^a-z0-9]+")
_RE_INLINE_CODE = re.compile(r"`([^`]+)`")
_RE_IMAGE = re.compile(r"!\[([^\]]*)\]\(([^)]+)\)")
_RE_LINK = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
_RE_WIKILINK = re.compile(r"\[\[([^\]|]+?)(?:\|([^\]]+))?\]\]")
_RE_BOLD_STAR = re.compile(r"\*\*(.+?)\*\*")
_RE_BOLD_UNDER = re.compile(r"__(.+?)__")
_RE_ITALIC_STAR = re.compile(r"\*(.+?)\*")
_RE_ITALIC_UNDER = re.compile(r"(?<!\w)_(.+?)_(?!\w)")
_RE_BQ_PREFIX = re.compile(r"^>\s?")
_RE_TABLE_CELL_SEP = re.compile(r"^[\s\-:]+$")


class RenderEngine:
    """Converts Markdown with wiki-links to self-contained HTML."""

    def render(
        self,
        markdown: str,
        *,
        title: str = "",
        theme: str = "default",
        resolve_links: bool = True,
        vault: Any = None,
    ) -> str:
        """Render Markdown to a complete, self-contained HTML document."""
        body_html = self._parse_markdown(markdown)

        if resolve_links:
            body_html = self._resolve_wiki_links(body_html, vault)

        toc_html = self._generate_toc(body_html)

        metadata_html = ""
        if title:
            metadata_html = f'<div class="metadata"><span><strong>{_esc(title)}</strong></span></div>'

        theme_css = get_theme(theme)

        return self._wrap_html(body_html, toc_html, title, theme_css, metadata_html)

    def _parse_markdown(self, md: str) -> str:
        """Convert Markdown to HTML. Custom state-machine parser (REGLA-372).

        All line patterns strip leading whitespace to tolerate LLM-generated
        markdown that uses indentation (e.g. " ## Heading").
        """
        lines = md.split("\n")
        output: list[str] = []
        i = 0

        while i < len(lines):
            line = lines[i]
            stripped = line.strip()

            # Fenced code blocks
            if stripped.startswith("```"):
                lang = stripped[3:].strip()
                code_lines: list[str] = []
                i += 1
                while i < len(lines) and not lines[i].strip().startswith("```"):
                    code_lines.append(lines[i])
                    i += 1
                i += 1  # skip closing ```
                code_content = _esc("\n".join(code_lines))
                lang_attr = f' class="language-{_esc(lang)}"' if lang else ""
                output.append(f"<pre><code{lang_attr}>{code_content}</code></pre>")
                continue

            # Headings — match on stripped line to tolerate leading whitespace
            heading_match = _RE_HEADING.match(stripped)
            if heading_match:
                level = len(heading_match.group(1))
                text = self._inline(heading_match.group(2).strip())
                slug = _RE_SLUG.sub("-", heading_match.group(2).lower()).strip("-")
                output.append(f'<h{level} id="{slug}">{text}</h{level}>')
                i += 1
                continue

            # Horizontal rule
            if _RE_HR.match(stripped):
                output.append("<hr>")
                i += 1
                continue

            # Blockquote
            if stripped.startswith(">"):
                bq_lines: list[str] = []
                while i < len(lines) and lines[i].strip().startswith(">"):
                    bq_lines.append(_RE_BQ_PREFIX.sub("", lines[i]))
                    i += 1
                inner = self._parse_markdown("\n".join(bq_lines))
                output.append(f"<blockquote>{inner}</blockquote>")
                continue

            # Unordered list
            if _RE_UL.match(stripped):
                items, i = self._parse_list(lines, i, ordered=False)
                output.append(items)
                continue

            # Ordered list
            if _RE_OL.match(stripped):
                items, i = self._parse_list(lines, i, ordered=True)
                output.append(items)
                continue

            # Table
            if "|" in line and i + 1 < len(lines) and _RE_TABLE_SEP.match(lines[i + 1]):
                table, i = self._parse_table(lines, i)
                output.append(table)
                continue

            # Empty line
            if not stripped:
                i += 1
                continue

            # Paragraph — collect contiguous non-empty, non-block-start lines
            para_lines: list[str] = []
            while i < len(lines) and lines[i].strip() and not self._is_block_start(lines[i]):
                para_lines.append(lines[i])
                i += 1
            if para_lines:
                text = self._inline(" ".join(para_lines))
                output.append(f"<p>{text}</p>")
            else:
                # Safety: if no pattern matched and paragraph collected nothing,
                # always advance to prevent infinite loop.
                i += 1

        return "\n".join(output)

    def _is_block_start(self, line: str) -> bool:
        """Check if a line starts a new block element."""
        s = line.strip()
        if s.startswith("#"):
            return True
        if s.startswith("```"):
            return True
        if s.startswith(">"):
            return True
        if _RE_UL.match(s):
            return True
        if _RE_OL.match(s):
            return True
        if _RE_HR.match(s):
            return True
        if "|" in s:
            return True
        return False

    def _parse_list(self, lines: list[str], start: int, ordered: bool) -> tuple[str, int]:
        """Parse a list block. Matches on stripped lines for LLM tolerance."""
        tag = "ol" if ordered else "ul"
        pattern = _RE_OL if ordered else _RE_UL
        items: list[str] = []
        i = start

        while i < len(lines):
            stripped = lines[i].strip()
            if not pattern.match(stripped):
                break
            content = pattern.sub("", stripped).strip()
            items.append(f"<li>{self._inline(content)}</li>")
            i += 1

        return f"<{tag}>{''.join(items)}</{tag}>", i

    def _parse_table(self, lines: list[str], start: int) -> tuple[str, int]:
        """Parse a Markdown pipe table."""
        i = start
        rows: list[list[str]] = []

        while i < len(lines) and "|" in lines[i]:
            cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
            if all(_RE_TABLE_CELL_SEP.match(c) for c in cells):
                i += 1
                continue
            rows.append(cells)
            i += 1

        if not rows:
            return "", i

        html_parts = ["<table>"]
        html_parts.append("<thead><tr>")
        for cell in rows[0]:
            html_parts.append(f"<th>{self._inline(cell)}</th>")
        html_parts.append("</tr></thead>")

        if len(rows) > 1:
            html_parts.append("<tbody>")
            for row in rows[1:]:
                html_parts.append("<tr>")
                for cell in row:
                    html_parts.append(f"<td>{self._inline(cell)}</td>")
                html_parts.append("</tr>")
            html_parts.append("</tbody>")

        html_parts.append("</table>")
        return "\n".join(html_parts), i

    def _inline(self, text: str) -> str:
        """Process inline Markdown elements: bold, italic, code, links, images, wiki-links."""
        text = _esc(text)
        text = _RE_INLINE_CODE.sub(r"<code>\1</code>", text)
        text = _RE_IMAGE.sub(r'<img src="\2" alt="\1">', text)
        text = _RE_LINK.sub(r'<a href="\2">\1</a>', text)
        text = _RE_WIKILINK.sub(
            lambda m: f'<span class="wiki-link" data-ref="{m.group(1).strip()}">{m.group(2) or m.group(1)}</span>',
            text,
        )
        text = _RE_BOLD_STAR.sub(r"<strong>\1</strong>", text)
        text = _RE_BOLD_UNDER.sub(r"<strong>\1</strong>", text)
        text = _RE_ITALIC_STAR.sub(r"<em>\1</em>", text)
        text = _RE_ITALIC_UNDER.sub(r"<em>\1</em>", text)
        return text

    def _resolve_wiki_links(self, html_content: str, vault: Any) -> str:
        """Replace wiki-link placeholders with resolved links or broken markers.

        REGLA-368: Broken links are visible (not hidden).
        """
        def _replace_link(match: re.Match) -> str:
            ref = match.group(1)
            display = match.group(2) or ref
            return f'<span class="vault-link-broken" title="Link: {_esc(ref)}">{display}</span>'

        html_content = re.sub(
            r'<span class="wiki-link" data-ref="([^"]+)">([^<]+)</span>',
            _replace_link,
            html_content,
        )
        return html_content

    def _generate_toc(self, html_content: str) -> str:
        """Generate table of contents from H1-H3."""
        headings = re.findall(r'<h([123])\s+id="([^"]+)">(.+?)</h\1>', html_content)
        if len(headings) < 2:
            return ""

        items: list[str] = []
        for level, slug, text in headings:
            clean_text = re.sub(r"<[^>]+>", "", text)
            indent = "  " * (int(level) - 1)
            items.append(f'{indent}<li><a href="#{slug}">{clean_text}</a></li>')

        return f'<nav class="toc"><h2>Contenido</h2><ul>{"".join(items)}</ul></nav>'

    def _wrap_html(
        self,
        body: str,
        toc: str,
        title: str,
        theme_css: str,
        metadata: str,
    ) -> str:
        """Build complete self-contained HTML document. REGLA-367."""
        safe_title = _esc(title) if title else "Report"
        return f"""<!DOCTYPE html>
<html lang="es">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{safe_title}</title>
    <style>{theme_css}</style>
</head>
<body>
    {metadata}
    {toc}
    <main>{body}</main>
</body>
</html>"""

    def render_rich(
        self,
        body_html: str,
        *,
        title: str = "",
        theme: str = "dark",
        extra_head: str = "",
    ) -> str:
        """Render rich HTML with JavaScript support (Chart.js, interactivity).

        Unlike render(), this takes raw HTML (not Markdown) and embeds it
        in a self-contained document with Chart.js loaded.
        Does NOT apply REGLA-370 (JS allowed for rich reports).
        """
        from datamirai_engine.render.charts import CHARTJS_LOADER

        theme_css = get_theme(theme)
        safe_title = _esc(title) if title else "Report"
        metadata_html = ""
        if title:
            metadata_html = f'<div class="metadata"><span><strong>{_esc(title)}</strong></span></div>'

        return f"""<!DOCTYPE html>
<html lang="es">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{safe_title}</title>
    <style>{theme_css}</style>
    {CHARTJS_LOADER}
    {extra_head}
</head>
<body>
    {metadata_html}
    <main>{body_html}</main>
</body>
</html>"""


def _esc(text: str) -> str:
    """Escape HTML entities."""
    return html.escape(text, quote=True)
