"""Knowledge Vault — Obsidian-inspired linked Markdown storage for agent data."""

from datamirai_engine.vault.parser import (
    build_note_markdown,
    extract_wiki_links,
    parse_frontmatter,
    slugify,
)
from datamirai_engine.vault.service import VaultNote, VaultService

__all__ = [
    "VaultService",
    "VaultNote",
    "parse_frontmatter",
    "extract_wiki_links",
    "build_note_markdown",
    "slugify",
]
