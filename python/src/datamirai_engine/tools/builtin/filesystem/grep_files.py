"""filesystem/grep — search file contents with regex."""

from __future__ import annotations

import os
import re
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)


class GrepFilesTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/grep",
        version="1.0.0",
        display_name="Grep (Search Content)",
        description="Searches file contents using regex. Returns matching lines with file paths and line numbers.",
        category="filesystem",
        icon="search",
        intents=[
            "buscar texto dentro de archivos",
            "encontrar donde se usa una funcion o variable",
            "buscar patrones con regex en el codigo",
        ],
        inputs=[
            ToolInput(name="pattern", type="string", required=True, description="Regex pattern to search for"),
        ],
        outputs=[
            ToolOutput(name="matches", type="array", description="List of {file, line, content} matches"),
            ToolOutput(name="files", type="array", description="Unique files with matches"),
            ToolOutput(name="count", type="number", description="Total number of matches"),
        ],
        config=[
            ConfigField(name="path", type="string", default=".", description="Directory or file to search in"),
            ConfigField(name="glob", type="string", default="", description="Glob filter for files (e.g. '*.py')"),
            ConfigField(name="max_results", type="number", default=100, description="Maximum matches to return"),
            ConfigField(name="case_insensitive", type="boolean", default=False, description="Case-insensitive search"),
            ConfigField(name="context_lines", type="number", default=0, description="Lines of context around each match"),
        ],
    )

    _BINARY_EXT = frozenset({
        ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".svg",
        ".woff", ".woff2", ".ttf", ".eot",
        ".zip", ".gz", ".tar", ".bz2", ".7z", ".rar",
        ".pdf", ".doc", ".docx", ".xls", ".xlsx",
        ".pyc", ".pyo", ".so", ".dylib", ".dll", ".exe",
        ".db", ".sqlite", ".sqlite3",
        ".mp3", ".mp4", ".wav", ".avi", ".mov",
    })

    _SKIP_DIRS = frozenset({
        "node_modules", ".git", "__pycache__", ".venv", "venv",
        "dist", "build", ".next", ".cache", ".tox", "egg-info",
    })

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        pattern_str = inputs["pattern"]
        base = os.path.abspath(config.get("path", "."))
        glob_filter = config.get("glob", "")
        max_results = int(config.get("max_results", 100))
        case_insensitive = config.get("case_insensitive", False)
        context_lines = int(config.get("context_lines", 0))

        flags = re.IGNORECASE if case_insensitive else 0
        try:
            regex = re.compile(pattern_str, flags)
        except re.error as exc:
            raise ValueError(f"Invalid regex pattern: {exc}") from exc

        import fnmatch

        matches: list[dict[str, Any]] = []
        files_seen: set[str] = set()

        target_files: list[str] = []
        if os.path.isfile(base):
            target_files = [base]
        else:
            for root, dirs, filenames in os.walk(base):
                dirs[:] = [d for d in dirs if d not in self._SKIP_DIRS]
                for fname in filenames:
                    ext = os.path.splitext(fname)[1].lower()
                    if ext in self._BINARY_EXT:
                        continue
                    if glob_filter and not fnmatch.fnmatch(fname, glob_filter):
                        continue
                    target_files.append(os.path.join(root, fname))

        for fpath in target_files:
            if len(matches) >= max_results:
                break
            try:
                with open(fpath, "r", encoding="utf-8", errors="replace") as f:
                    lines = f.readlines()
            except (OSError, PermissionError):
                continue

            for i, line in enumerate(lines):
                if len(matches) >= max_results:
                    break
                if regex.search(line):
                    files_seen.add(fpath)
                    entry: dict[str, Any] = {
                        "file": fpath,
                        "line": i + 1,
                        "content": line.rstrip(),
                    }
                    if context_lines > 0:
                        start = max(0, i - context_lines)
                        end = min(len(lines), i + context_lines + 1)
                        ctx = [ln.rstrip() for ln in lines[start:end]]
                        entry["context"] = ctx
                    matches.append(entry)

        return {
            "matches": matches,
            "files": sorted(files_seen),
            "count": len(matches),
        }
