"""Vault Write tool — writes Markdown notes to the Knowledge Vault."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import BaseTool, ConfigField, ToolInput, ToolOutput, ToolSpec
from datamirai_engine.vault.parser import slugify


class VaultWriteTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/vault_write",
        version="1.0.0",
        display_name="Vault Write",
        description="Escribe una nota Markdown al Knowledge Vault del environment",
        category="data",
        icon="file-plus",
        intents=[
            "guardar datos scrapeados como nota persistente en el vault",
            "crear nota de analisis que referencia notas existentes",
            "almacenar reporte con wiki-links para trazabilidad",
            "persistir conocimiento entre sesiones del agente",
        ],
        inputs=[
            ToolInput(name="content", type="string", required=True,
                      description="Contenido Markdown de la nota"),
            ToolInput(name="title", type="string", required=False,
                      description="Titulo de la nota (default: generado del contenido)"),
            ToolInput(name="extra_frontmatter", type="object", required=False,
                      description="Campos adicionales para el frontmatter YAML"),
        ],
        outputs=[
            ToolOutput(name="path", type="string",
                       description="Path de la nota creada en el vault"),
            ToolOutput(name="title", type="string"),
            ToolOutput(name="outlinks", type="array",
                       description="Wiki-links encontrados en el contenido"),
        ],
        config=[
            ConfigField(name="folder", type="string", default="vault/notes",
                        description="Carpeta destino dentro del vault"),
            ConfigField(name="filename_pattern", type="string",
                        default="{date}-{slug}",
                        description="Patron del nombre: {date}, {slug}, {timestamp}, {session_id}"),
            ConfigField(name="tags", type="string", default="",
                        description="Tags por defecto (separados por coma)"),
            ConfigField(name="overwrite", type="boolean", default=False),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        # REGLA-352: vault must be available
        if not context or not getattr(context, "vault", None):
            raise RuntimeError(
                "Knowledge Vault requiere un environment con storage configurado. "
                "Configura recursos de storage y DB en el environment."
            )

        vault = context.vault
        content = inputs.get("content", "")
        # Handle case where content comes as dict (e.g., from trigger output)
        if isinstance(content, dict):
            import json
            content = json.dumps(content, indent=2, ensure_ascii=False, default=str)
        elif not isinstance(content, str):
            content = str(content)
        title = inputs.get("title", "") or _extract_title(content)
        if isinstance(title, dict):
            title = str(title.get("user_input", title))
        elif not isinstance(title, str):
            title = str(title)
        extra_fm = inputs.get("extra_frontmatter") or {}

        # Build frontmatter
        config_tags = [t.strip() for t in config.get("tags", "").split(",") if t.strip()]
        extra_tags = extra_fm.pop("tags", [])
        if isinstance(extra_tags, str):
            extra_tags = [t.strip() for t in extra_tags.split(",") if t.strip()]
        all_tags = list(dict.fromkeys(config_tags + extra_tags))  # dedupe preserving order

        frontmatter: dict[str, Any] = {
            "title": title,
            "tags": all_tags,
            **extra_fm,
        }

        # REGLA-349: inject agent_id and session_id from context
        if context.node_id:
            frontmatter.setdefault("node_id", context.node_id)
        if context.session_id:
            frontmatter.setdefault("session_id", context.session_id)

        # Generate filename — REGLA-348
        folder = config.get("folder", "vault/notes").rstrip("/")
        pattern = config.get("filename_pattern", "{date}-{slug}")
        filename = _generate_filename(pattern, title, context)
        path = f"{folder}/{filename}.md"

        # Handle collision
        overwrite = config.get("overwrite", False)
        if isinstance(overwrite, str):
            overwrite = overwrite.lower() in ("true", "1", "yes")

        if not overwrite:
            # Try to find a non-colliding name
            try:
                await vault.read_note(path)
                # Note exists — append suffix
                for i in range(2, 100):
                    alt_path = f"{folder}/{filename}-{i}.md"
                    try:
                        await vault.read_note(alt_path)
                    except FileNotFoundError:
                        path = alt_path
                        break
                else:
                    raise FileExistsError(f"Too many collisions for {path}")
            except FileNotFoundError:
                pass  # Path is available

        note = await vault.write_note(
            path=path,
            title=title,
            content=content,
            frontmatter=frontmatter,
            overwrite=overwrite,
        )

        return {
            "path": note.path,
            "title": note.title,
            "outlinks": note.outlinks,
        }


def _extract_title(content: str) -> str:
    """Extract title from first H1 or first line."""
    for line in content.strip().splitlines():
        line = line.strip()
        if line.startswith("# "):
            return line[2:].strip()
        if line:
            return line[:80]
    return "Untitled"


def _generate_filename(pattern: str, title: str, context: Any) -> str:
    """Generate filename from pattern. REGLA-348: slugified, max 80 chars."""
    now = datetime.now(timezone.utc)
    replacements = {
        "{date}": now.strftime("%Y-%m-%d"),
        "{timestamp}": now.strftime("%Y%m%d-%H%M%S"),
        "{slug}": slugify(title, max_len=60),
        "{session_id}": (context.session_id or "no-session")[:12] if context else "no-ctx",
    }
    result = pattern
    for key, value in replacements.items():
        result = result.replace(key, value)

    # Final slugify to ensure clean filename
    return slugify(result, max_len=80)
