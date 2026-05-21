"""filesystem/edit_file — exact search-and-replace editing."""

from __future__ import annotations

import os
from typing import Any

from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.base import (
    BaseTool,
    ConfigField,
    ToolInput,
    ToolOutput,
    ToolSpec,
)


class EditFileTool(BaseTool):
    spec = ToolSpec(
        tool_type="filesystem/edit_file",
        version="1.0.0",
        display_name="Edit File",
        description="Performs exact string replacement in a file. The old_string must match exactly (including whitespace). By default replaces only the first occurrence.",
        category="filesystem",
        icon="file-edit",
        intents=[
            "editar un archivo reemplazando texto exacto",
            "modificar una parte especifica de un archivo",
            "hacer search-and-replace en un archivo",
        ],
        inputs=[
            ToolInput(name="path", type="string", required=True, description="Path to the file to edit"),
            ToolInput(name="old_string", type="string", required=True, description="Exact string to find"),
            ToolInput(name="new_string", type="string", required=True, description="Replacement string"),
        ],
        outputs=[
            ToolOutput(name="path", type="string", description="Resolved absolute path"),
            ToolOutput(name="replacements", type="number", description="Number of replacements made"),
            ToolOutput(name="diff", type="string", description="Summary of changes"),
        ],
        config=[
            ConfigField(name="replace_all", type="boolean", default=False, description="Replace all occurrences instead of just the first"),
        ],
    )

    async def execute(
        self,
        inputs: dict[str, Any],
        config: dict[str, Any],
        context: ExecutionContext | None,
    ) -> dict[str, Any]:
        path = os.path.abspath(inputs["path"])
        old_string = inputs["old_string"]
        new_string = inputs["new_string"]
        replace_all = config.get("replace_all", False)

        if not os.path.isfile(path):
            raise FileNotFoundError(f"File not found: {path}")

        with open(path, "r", encoding="utf-8", errors="replace") as f:
            content = f.read()

        if old_string == new_string:
            raise ValueError("old_string and new_string are identical")

        count = content.count(old_string)
        if count == 0:
            raise ValueError(f"old_string not found in {path}")

        if not replace_all and count > 1:
            raise ValueError(
                f"old_string found {count} times in {path}. "
                "Provide more context to make it unique, or set replace_all=true."
            )

        if replace_all:
            new_content = content.replace(old_string, new_string)
            replacements = count
        else:
            new_content = content.replace(old_string, new_string, 1)
            replacements = 1

        with open(path, "w", encoding="utf-8") as f:
            f.write(new_content)

        old_preview = old_string[:80].replace("\n", "\\n")
        new_preview = new_string[:80].replace("\n", "\\n")
        diff = f"-  {old_preview}\n+  {new_preview}\n({replacements} replacement(s))"

        return {
            "path": path,
            "replacements": replacements,
            "diff": diff,
        }
