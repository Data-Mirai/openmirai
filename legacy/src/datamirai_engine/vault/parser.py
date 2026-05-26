"""Vault parser — frontmatter, wiki-links, and filename utilities."""

from __future__ import annotations

import re
import unicodedata
from typing import Any

import yaml


def parse_frontmatter(md_text: str) -> tuple[dict[str, Any], str]:
    """Split a Markdown file into YAML frontmatter and body content.

    Returns (frontmatter_dict, body_string).
    If YAML is invalid or absent, returns ({}, full_text).  # REGLA-343
    """
    if not md_text.startswith("---"):
        return {}, md_text

    # Find closing ---
    end_idx = md_text.find("\n---", 3)
    if end_idx == -1:
        return {}, md_text

    yaml_block = md_text[4:end_idx]  # skip opening ---\n
    body = md_text[end_idx + 4:]  # skip \n---
    if body.startswith("\n"):
        body = body[1:]

    try:
        frontmatter = yaml.safe_load(yaml_block)
        if not isinstance(frontmatter, dict):
            return {}, md_text
        return frontmatter, body
    except yaml.YAMLError:
        return {}, md_text


# REGLA-344: [[reference]] or [[reference|alias]]
_WIKI_LINK_RE = re.compile(r"\[\[([^\]|]+?)(?:\|([^\]]+))?\]\]")


def extract_wiki_links(content: str) -> list[tuple[str, str | None]]:
    """Extract wiki-links from Markdown content.

    Returns list of (reference, alias_or_None).
    Example: [[fed-news]] -> [("fed-news", None)]
             [[fed-news|Federal Reserve]] -> [("fed-news", "Federal Reserve")]
    """
    results: list[tuple[str, str | None]] = []
    seen: set[str] = set()
    for match in _WIKI_LINK_RE.finditer(content):
        ref = match.group(1).strip()
        alias = match.group(2)
        if alias:
            alias = alias.strip()
        if ref and ref not in seen:
            seen.add(ref)
            results.append((ref, alias))
    return results


def build_note_markdown(frontmatter: dict[str, Any], content: str) -> str:
    """Build a complete Markdown note with YAML frontmatter."""
    if frontmatter:
        yaml_str = yaml.dump(
            frontmatter,
            default_flow_style=False,
            allow_unicode=True,
            sort_keys=False,
        ).rstrip()
        return f"---\n{yaml_str}\n---\n\n{content}"
    return content


def slugify(text: str, max_len: int = 80) -> str:
    """Convert text to a URL/filename-safe slug.

    Lowercase, strip accents, replace spaces/special chars with hyphens.
    """
    # Normalize unicode
    text = unicodedata.normalize("NFKD", text)
    # Remove accents
    text = "".join(c for c in text if not unicodedata.combining(c))
    # Lowercase
    text = text.lower()
    # Replace non-alphanumeric with hyphens
    text = re.sub(r"[^a-z0-9]+", "-", text)
    # Strip leading/trailing hyphens
    text = text.strip("-")
    # Collapse multiple hyphens
    text = re.sub(r"-{2,}", "-", text)
    # Truncate
    if len(text) > max_len:
        text = text[:max_len].rstrip("-")
    return text or "untitled"
