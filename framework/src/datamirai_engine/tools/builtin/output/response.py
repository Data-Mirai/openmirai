"""Response tool — terminal node that builds the final session result.

Collects data from previous nodes via data_map, applies a template,
and produces the structured output that becomes the session result.
"""

from __future__ import annotations

import json
import re
from typing import Any

from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)


class ResponseTool(BaseTool):
    spec = ToolSpec(
        tool_type="output/response",
        version="1.0.0",
        display_name="Response",
        description="Nodo terminal que construye el resultado final de la sesion",
        category="output",
        icon="flag",
        intents=[
            "entregar el resultado final del agente al usuario",
            "formatear la salida como reporte, lista, JSON, markdown o texto",
            "todo grafo debe terminar con este nodo para producir un resultado visible",
        ],
        inputs=[
            ToolInput(
                name="data",
                type="any",
                required=False,
                description="Datos del paso anterior (auto-conectado via data_map)",
            ),
        ],
        outputs=[
            ToolOutput(name="result", type="string", description="Resultado formateado"),
            ToolOutput(name="format", type="string", description="Formato de salida usado"),
            ToolOutput(name="raw_data", type="object", description="Datos crudos recibidos"),
        ],
        config=[
            ConfigField(
                name="template",
                type="string",
                default="",
                description="Template de respuesta. Usa ${campo} para interpolar datos.",
            ),
            ConfigField(
                name="format",
                type="select",
                default="markdown",
                description="Formato de presentacion del resultado",
                options=["markdown", "text", "json", "bullets", "report", "html", "rich_html"],
            ),
            ConfigField(
                name="title",
                type="string",
                default="",
                description="Titulo del resultado (opcional)",
            ),
            ConfigField(
                name="theme",
                type="select",
                default="default",
                description="Tema visual del HTML generado (solo aplica con format=html)",
                options=["default", "dark", "minimal", "report"],
            ),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: Any,
    ) -> dict[str, Any]:
        raw_data = inputs.get("data", inputs)
        template = config.get("template", "")
        fmt = config.get("format", "markdown")
        title = config.get("title", "")

        # Build result from template or auto-format
        if template:
            result = self._apply_template(template, raw_data)
            if fmt == "html":
                result = self._render_html(result, title, config, context)
            elif fmt == "rich_html":
                result = self._render_rich_html(result, title, config, context)
        else:
            result = self._auto_format(raw_data, fmt, title, config, context)

        return {
            "result": result,
            "format": fmt,
            "raw_data": raw_data if isinstance(raw_data, dict) else {"data": raw_data},
        }

    def _apply_template(self, template: str, data: Any) -> str:
        """Replace ${key} placeholders with values from data."""
        if not isinstance(data, dict):
            return template.replace("${data}", str(data))

        def _replace(m: re.Match) -> str:
            key = m.group(1)
            val = data.get(key)
            if val is None:
                return m.group(0)
            if isinstance(val, str):
                return val
            return json.dumps(val, ensure_ascii=False, default=str)

        return re.sub(r"\$\{([^}]+)\}", _replace, template)

    def _auto_format(self, data: Any, fmt: str, title: str, config: dict | None = None, context: Any = None) -> str:
        """Auto-format data based on selected format."""
        header = f"# {title}\n\n" if title else ""

        if fmt == "json":
            content = json.dumps(
                data, indent=2, ensure_ascii=False, default=str
            )
            return f"{header}{content}"

        # Convert data to string content
        if isinstance(data, str):
            content = data
        elif isinstance(data, dict):
            # Extract the most meaningful value
            content = self._extract_content(data)
        elif isinstance(data, list):
            content = "\n".join(str(item) for item in data)
        else:
            content = str(data)

        if fmt == "bullets":
            lines = [l.strip() for l in content.split("\n") if l.strip()]
            body = "\n".join(f"- {line}" for line in lines)
            return f"{header}{body}"

        if fmt == "report":
            return f"{header}---\n\n{content}\n\n---"

        if fmt == "text":
            return f"{title}\n\n{content}" if title else content

        # html — render markdown to self-contained HTML
        if fmt == "html":
            md_content = f"{header}{content}" if header else content
            return self._render_html(md_content, title, config or {}, context)

        # rich_html — self-contained HTML with JS + Chart.js + data API support
        if fmt == "rich_html":
            return self._render_rich_html(content, title, config or {}, context)

        # markdown (default)
        return f"{header}{content}"

    def _render_html(self, markdown: str, title: str, config: dict, context: Any) -> str:
        """Render Markdown to self-contained HTML using the RenderEngine."""
        from datamirai_engine.render.engine import RenderEngine

        engine = RenderEngine()
        vault = getattr(context, "vault", None) if context else None
        theme = config.get("theme", "default")

        return engine.render(
            markdown,
            title=title,
            theme=theme,
            resolve_links=True,
            vault=vault,
        )

    def _render_rich_html(self, body_html: str, title: str, config: dict, context: Any = None) -> str:
        """Render rich HTML with JS support (Chart.js, data API fetch)."""
        from datamirai_engine.render.engine import RenderEngine

        engine = RenderEngine()
        theme = config.get("theme", "dark")

        # Inject data API config so the HTML can fetch live data
        env_id = getattr(context, "environment_id", None) or ""
        api_base = getattr(context, "api_base", "http://localhost:8000")
        data_api_script = (
            f'<script>'
            f'window.__DATAMIRAI_API_BASE__ = "{api_base}";'
            f'window.__DATAMIRAI_ENV_ID__ = "{env_id}";'
            f'window.__DATAMIRAI_DATA_URL__ = "{api_base}/api/environments/{env_id}/data";'
            f'async function fetchData(table, params) {{'
            f'  const qs = new URLSearchParams(params || {{}}).toString();'
            f'  const url = window.__DATAMIRAI_DATA_URL__ + "/" + table + (qs ? "?" + qs : "");'
            f'  const res = await fetch(url);'
            f'  return res.json();'
            f'}}'
            f'</script>'
        )
        return engine.render_rich(body_html, title=title, theme=theme, extra_head=data_api_script)

    def _extract_content(self, data: dict) -> str:
        """Extract the most meaningful string from a dict output."""
        # Common keys that contain the main result
        for key in ("result", "output", "text", "content", "response", "summary"):
            if key in data and isinstance(data[key], str):
                return data[key]
        # Fallback: pretty-print the whole dict
        return json.dumps(data, indent=2, ensure_ascii=False, default=str)
