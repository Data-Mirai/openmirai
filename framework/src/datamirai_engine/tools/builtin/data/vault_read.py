"""Vault Read tool — reads and queries notes from the Knowledge Vault."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import BaseTool, ConfigField, ToolInput, ToolOutput, ToolSpec


class VaultReadTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/vault_read",
        version="1.0.0",
        display_name="Vault Read",
        description="Lee notas del Knowledge Vault por tags, carpeta, busqueda o backlinks",
        category="data",
        icon="file-search",
        intents=[
            "leer notas previas del vault para contexto",
            "buscar notas por tags o carpeta",
            "obtener backlinks de una nota para descubrir conexiones",
            "cargar notas recientes para alimentar analisis del LLM",
        ],
        inputs=[
            ToolInput(name="query", type="string", required=False,
                      description="Busqueda full-text en titulos y contenido"),
            ToolInput(name="note_path", type="string", required=False,
                      description="Path especifico para leer una nota o sus backlinks"),
        ],
        outputs=[
            ToolOutput(name="notes", type="array",
                       description="Lista de notas encontradas"),
            ToolOutput(name="count", type="number"),
        ],
        config=[
            ConfigField(name="mode", type="select", default="search",
                        options=["search", "read", "backlinks", "recent"],
                        description="search=query, read=nota especifica, backlinks=referencias, recent=ultimas N"),
            ConfigField(name="folder", type="string", default="",
                        description="Filtrar por carpeta (ej: vault/scrapes)"),
            ConfigField(name="tags", type="string", default="",
                        description="Filtrar por tags (separados por coma)"),
            ConfigField(name="since", type="select", default="any",
                        options=["any", "today", "week", "month"],
                        description="Filtrar por antiguedad"),
            ConfigField(name="limit", type="number", default=10),
            ConfigField(name="include_content", type="boolean", default=True,
                        description="Incluir contenido completo de cada nota"),
        ],
    )

    # REGLA-350: max content per note to protect LLM context window
    MAX_CONTENT_LENGTH = 10_000

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        # REGLA-352
        if not context or not getattr(context, "vault", None):
            raise RuntimeError(
                "Knowledge Vault requiere un environment con storage configurado. "
                "Configura recursos de storage y DB en el environment."
            )

        vault = context.vault
        mode = config.get("mode", "search")
        note_path = inputs.get("note_path", "")
        query = inputs.get("query", "")
        limit = int(config.get("limit", 10))
        include_content = config.get("include_content", True)
        if isinstance(include_content, str):
            include_content = include_content.lower() in ("true", "1", "yes")

        notes = []

        if mode == "read":
            if not note_path:
                raise ValueError("mode=read requiere note_path")
            note = await vault.read_note(note_path)
            notes = [note]

        elif mode == "backlinks":
            # REGLA-351
            if not note_path:
                raise ValueError("mode=backlinks requiere note_path")
            notes = await vault.backlinks(note_path)

        elif mode == "recent":
            notes = await vault.search(limit=limit)

        else:  # search
            tag_list = [t.strip() for t in config.get("tags", "").split(",") if t.strip()]
            folder = config.get("folder", "") or None
            since = _resolve_since(config.get("since", "any"))

            notes = await vault.search(
                tags=tag_list or None,
                query=query or None,
                folder=folder,
                since=since,
                limit=limit,
            )

        # Load content if needed
        if include_content:
            enriched = []
            for note in notes:
                if not note.content:
                    try:
                        full = await vault.read_note(note.path)
                        note = full
                    except FileNotFoundError:
                        pass
                # REGLA-350: truncate
                if len(note.content) > self.MAX_CONTENT_LENGTH:
                    note.content = note.content[:self.MAX_CONTENT_LENGTH] + "\n\n[... truncado]"
                enriched.append(note)
            notes = enriched

        result_notes = [
            {
                "path": n.path,
                "title": n.title,
                "content": n.content if include_content else "",
                "tags": n.frontmatter.get("tags", []),
                "created_at": n.created_at,
                "updated_at": n.updated_at,
                "outlinks": n.outlinks,
            }
            for n in notes
        ]

        return {"notes": result_notes, "count": len(result_notes)}


def _resolve_since(since: str) -> str | None:
    """Convert since filter to ISO date string."""
    if since == "any" or not since:
        return None

    now = datetime.now(timezone.utc)
    deltas = {
        "today": timedelta(days=1),
        "week": timedelta(weeks=1),
        "month": timedelta(days=30),
    }
    delta = deltas.get(since)
    if not delta:
        return None

    return (now - delta).isoformat()
