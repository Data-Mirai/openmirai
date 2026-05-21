"""VaultService — Intelligence layer over StorageResource + DBResource.

Manages Markdown notes with YAML frontmatter and [[wiki-links]].
Files live in storage (vault/ prefix), index lives in agent.db.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from datamirai_engine.vault.parser import (
    build_note_markdown,
    extract_wiki_links,
    parse_frontmatter,
    slugify,
)


@dataclass
class VaultNote:
    """A note in the Knowledge Vault."""

    path: str
    title: str
    content: str = ""
    frontmatter: dict[str, Any] = field(default_factory=dict)
    outlinks: list[str] = field(default_factory=list)
    created_at: str = ""
    updated_at: str = ""


class VaultService:
    """Knowledge Vault — Obsidian-inspired linked Markdown notes.

    Uses StorageResource for .md files and DBResource for the index.
    The index is DERIVED from files — reindex() rebuilds it completely (REGLA-342).
    """

    VAULT_PREFIX = "vault/"

    def __init__(self, storage: Any, db: Any) -> None:
        self._storage = storage
        self._db = db
        self._schema_ensured = False

    async def _ensure_schema(self) -> None:
        """Create vault tables if they don't exist. Idempotent."""
        if self._schema_ensured:
            return

        await self._db.execute(
            """CREATE TABLE IF NOT EXISTS vault_notes (
                path TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                folder TEXT NOT NULL DEFAULT '',
                tags TEXT NOT NULL DEFAULT '[]',
                frontmatter TEXT NOT NULL DEFAULT '{}',
                agent_id TEXT,
                session_id TEXT,
                content_length INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )"""
        )
        await self._db.execute(
            """CREATE TABLE IF NOT EXISTS vault_links (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_path TEXT NOT NULL,
                target_ref TEXT NOT NULL,
                target_path TEXT,
                FOREIGN KEY (source_path) REFERENCES vault_notes(path) ON DELETE CASCADE
            )"""
        )
        # Indices for fast queries
        for idx_sql in [
            "CREATE INDEX IF NOT EXISTS idx_vn_folder ON vault_notes(folder)",
            "CREATE INDEX IF NOT EXISTS idx_vn_agent ON vault_notes(agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_vn_created ON vault_notes(created_at)",
            "CREATE INDEX IF NOT EXISTS idx_vl_source ON vault_links(source_path)",
            "CREATE INDEX IF NOT EXISTS idx_vl_target ON vault_links(target_path)",
        ]:
            await self._db.execute(idx_sql)

        self._schema_ensured = True

    # --- Write ---

    async def write_note(
        self,
        path: str,
        title: str,
        content: str,
        frontmatter: dict[str, Any] | None = None,
        *,
        overwrite: bool = False,
    ) -> VaultNote:
        """Write a Markdown note to storage and index it.

        REGLA-345: overwrite=False (default) fails if note already exists.
        REGLA-346: auto-injects created_at, updated_at.
        REGLA-347: output is standard .md UTF-8.
        """
        await self._ensure_schema()

        frontmatter = dict(frontmatter) if frontmatter else {}
        now = datetime.now(timezone.utc).isoformat()

        # Check existence
        if not overwrite:
            existing = await self._db.fetch_one(
                "SELECT path FROM vault_notes WHERE path = ?", (path,)
            )
            if existing:
                raise FileExistsError(f"Note already exists: {path}. Use overwrite=True to replace.")

        # Ensure timestamps in frontmatter (REGLA-346)
        if "created_at" not in frontmatter:
            frontmatter["created_at"] = now
        frontmatter["updated_at"] = now

        if "title" not in frontmatter:
            frontmatter["title"] = title

        # Build markdown and write to storage
        md_bytes = build_note_markdown(frontmatter, content).encode("utf-8")
        await self._storage.put(path, md_bytes)

        # Extract links
        links = extract_wiki_links(content)
        outlink_refs = [ref for ref, _ in links]

        # Compute folder from path
        parts = path.rsplit("/", 1)
        folder = parts[0] if len(parts) > 1 else ""

        # Extract tags
        tags = frontmatter.get("tags", [])
        if isinstance(tags, str):
            tags = [t.strip() for t in tags.split(",") if t.strip()]
        tags_json = json.dumps(tags, ensure_ascii=False)
        fm_json = json.dumps(frontmatter, ensure_ascii=False, default=str)

        # Upsert vault_notes
        await self._db.execute(
            """INSERT INTO vault_notes (path, title, folder, tags, frontmatter,
                agent_id, session_id, content_length, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(path) DO UPDATE SET
                    title=excluded.title, folder=excluded.folder, tags=excluded.tags,
                    frontmatter=excluded.frontmatter, agent_id=excluded.agent_id,
                    session_id=excluded.session_id, content_length=excluded.content_length,
                    updated_at=excluded.updated_at""",
            (
                path, title, folder, tags_json, fm_json,
                frontmatter.get("agent_id"), frontmatter.get("session_id"),
                len(md_bytes), frontmatter.get("created_at", now), now,
            ),
        )

        # Replace links
        await self._db.execute(
            "DELETE FROM vault_links WHERE source_path = ?", (path,)
        )
        for ref, _ in links:
            resolved = await self._resolve_link(ref, exclude_path=path)
            await self._db.execute(
                "INSERT INTO vault_links (source_path, target_ref, target_path) VALUES (?, ?, ?)",
                (path, ref, resolved),
            )

        return VaultNote(
            path=path,
            title=title,
            content=content,
            frontmatter=frontmatter,
            outlinks=outlink_refs,
            created_at=frontmatter.get("created_at", now),
            updated_at=now,
        )

    # --- Read ---

    async def read_note(self, path: str) -> VaultNote:
        """Read a note from storage, parse frontmatter and links."""
        try:
            data = await self._storage.get(path)
        except (FileNotFoundError, Exception) as exc:
            raise FileNotFoundError(f"Note not found: {path}") from exc

        md_text = data.decode("utf-8")
        frontmatter, body = parse_frontmatter(md_text)
        links = extract_wiki_links(body)

        return VaultNote(
            path=path,
            title=frontmatter.get("title", _title_from_content(body)),
            content=body,
            frontmatter=frontmatter,
            outlinks=[ref for ref, _ in links],
            created_at=frontmatter.get("created_at", ""),
            updated_at=frontmatter.get("updated_at", ""),
        )

    # --- Search ---

    async def search(
        self,
        *,
        tags: list[str] | None = None,
        query: str | None = None,
        folder: str | None = None,
        since: str | None = None,
        limit: int = 20,
    ) -> list[VaultNote]:
        """Query the vault index. Returns metadata-only notes (no content from storage)."""
        await self._ensure_schema()

        conditions: list[str] = []
        params: list[Any] = []

        if tags:
            for tag in tags:
                conditions.append("tags LIKE ?")
                params.append(f'%"{tag}"%')

        if query:
            conditions.append("(title LIKE ? OR path LIKE ?)")
            params.extend([f"%{query}%", f"%{query}%"])

        if folder:
            conditions.append("folder = ?")
            params.append(folder)

        if since:
            conditions.append("created_at >= ?")
            params.append(since)

        where = " AND ".join(conditions) if conditions else "1=1"
        sql = f"SELECT * FROM vault_notes WHERE {where} ORDER BY created_at DESC LIMIT ?"
        params.append(limit)

        rows = await self._db.fetch_all(sql, tuple(params))
        return [_row_to_note(row) for row in rows]

    # --- Links ---

    async def backlinks(self, note_path: str) -> list[VaultNote]:
        """Notes that reference this note via [[wiki-link]]."""
        await self._ensure_schema()

        # Match by full path or by the note's filename slug
        slug = note_path.rsplit("/", 1)[-1].replace(".md", "")
        rows = await self._db.fetch_all(
            """SELECT DISTINCT vn.* FROM vault_notes vn
               JOIN vault_links vl ON vn.path = vl.source_path
               WHERE vl.target_path = ? OR vl.target_ref = ?""",
            (note_path, slug),
        )
        return [_row_to_note(row) for row in rows]

    async def outlinks(self, note_path: str) -> list[VaultNote]:
        """Notes referenced by this note."""
        await self._ensure_schema()

        rows = await self._db.fetch_all(
            """SELECT DISTINCT vn.* FROM vault_notes vn
               JOIN vault_links vl ON vn.path = vl.target_path
               WHERE vl.source_path = ?""",
            (note_path,),
        )
        return [_row_to_note(row) for row in rows]

    # --- Delete ---

    async def delete_note(self, path: str) -> None:
        """Delete note from storage and index."""
        await self._ensure_schema()

        try:
            await self._storage.delete(path)
        except Exception:
            pass  # File might not exist in storage

        await self._db.execute("DELETE FROM vault_notes WHERE path = ?", (path,))
        # vault_links cascade on delete

    # --- Reindex ---

    async def reindex(self) -> int:
        """Rebuild index completely from files in storage. REGLA-342."""
        await self._ensure_schema()

        # Clear existing index
        await self._db.execute("DELETE FROM vault_links")
        await self._db.execute("DELETE FROM vault_notes")

        # List all .md files in vault/
        keys = await self._storage.list_keys(self.VAULT_PREFIX)
        md_keys = [k for k in keys if k.endswith(".md")]

        count = 0
        for key in md_keys:
            try:
                data = await self._storage.get(key)
                md_text = data.decode("utf-8")
                frontmatter, body = parse_frontmatter(md_text)
                title = frontmatter.get("title", _title_from_content(body))
                links = extract_wiki_links(body)

                tags = frontmatter.get("tags", [])
                if isinstance(tags, str):
                    tags = [t.strip() for t in tags.split(",") if t.strip()]

                parts = key.rsplit("/", 1)
                folder = parts[0] if len(parts) > 1 else ""

                await self._db.execute(
                    """INSERT OR REPLACE INTO vault_notes
                       (path, title, folder, tags, frontmatter, agent_id, session_id,
                        content_length, created_at, updated_at)
                       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                    (
                        key, title, folder,
                        json.dumps(tags, ensure_ascii=False),
                        json.dumps(frontmatter, ensure_ascii=False, default=str),
                        frontmatter.get("agent_id"),
                        frontmatter.get("session_id"),
                        len(data),
                        frontmatter.get("created_at", ""),
                        frontmatter.get("updated_at", ""),
                    ),
                )

                for ref, _ in links:
                    resolved = await self._resolve_link(ref, exclude_path=key)
                    await self._db.execute(
                        "INSERT INTO vault_links (source_path, target_ref, target_path) VALUES (?, ?, ?)",
                        (key, ref, resolved),
                    )

                count += 1
            except Exception:
                continue  # Skip broken files

        return count

    # --- Tags ---

    async def list_tags(self) -> list[tuple[str, int]]:
        """Return unique tags with note counts."""
        await self._ensure_schema()

        rows = await self._db.fetch_all(
            "SELECT tags FROM vault_notes WHERE tags != '[]'"
        )
        tag_counts: dict[str, int] = {}
        for row in rows:
            try:
                tags = json.loads(row["tags"])
                for tag in tags:
                    tag_counts[tag] = tag_counts.get(tag, 0) + 1
            except (json.JSONDecodeError, TypeError):
                continue

        return sorted(tag_counts.items(), key=lambda x: -x[1])

    # --- Internal ---

    async def _resolve_link(self, ref: str, exclude_path: str = "") -> str | None:
        """Resolve a wiki-link reference to a note path (shortest-path matching)."""
        # Try exact path match first
        if not ref.endswith(".md"):
            ref_md = ref + ".md"
        else:
            ref_md = ref

        # Try with vault prefix
        candidates = [
            ref_md,
            f"{self.VAULT_PREFIX}{ref_md}",
        ]

        for candidate in candidates:
            row = await self._db.fetch_one(
                "SELECT path FROM vault_notes WHERE path = ? AND path != ?",
                (candidate, exclude_path),
            )
            if row:
                return row["path"]

        # Try suffix match (shortest-path, like Obsidian)
        row = await self._db.fetch_one(
            "SELECT path FROM vault_notes WHERE path LIKE ? AND path != ? LIMIT 1",
            (f"%/{ref_md}", exclude_path),
        )
        if row:
            return row["path"]

        # Try slug match
        slug = slugify(ref)
        row = await self._db.fetch_one(
            "SELECT path FROM vault_notes WHERE path LIKE ? AND path != ? LIMIT 1",
            (f"%{slug}%", exclude_path),
        )
        if row:
            return row["path"]

        return None  # Broken link — indexed but unresolved


def _title_from_content(body: str) -> str:
    """Extract title from first H1 or first line of content."""
    for line in body.strip().splitlines():
        line = line.strip()
        if line.startswith("# "):
            return line[2:].strip()
        if line:
            return line[:100]
    return "Untitled"


def _row_to_note(row: dict) -> VaultNote:
    """Convert a database row to a VaultNote (metadata only, no content)."""
    fm = {}
    try:
        fm = json.loads(row.get("frontmatter", "{}"))
    except (json.JSONDecodeError, TypeError):
        pass

    return VaultNote(
        path=row["path"],
        title=row.get("title", ""),
        content="",  # Content not loaded from storage in search results
        frontmatter=fm,
        outlinks=[],
        created_at=row.get("created_at", ""),
        updated_at=row.get("updated_at", ""),
    )
