"""autonomy_levels — defines how the agentic loop behaves per autonomy level.

L1 Assisted:    1 tool per turn, confirm all writes
L2 Copilot:     N tools per turn, no confirm on reads, confirm on writes
L3 Autopilot:   Full autonomy, human can interrupt
L4 Self-Driving: Continuous loop toward goals
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


# Tools that MODIFY state (files, git, system)
WRITE_TOOLS = frozenset({
    "filesystem/write_file",
    "filesystem/edit_file",
    "filesystem/move",
    "filesystem/copy",
    "filesystem/delete",
    "filesystem/mkdir",
    "system/bash",
    "git/commit",
    "data/db_write",
    "data/storage_write",
    "data/vault_write",
    "data/entity_upsert",
})

# Tools that only READ state
READ_TOOLS = frozenset({
    "filesystem/read_file",
    "filesystem/glob",
    "filesystem/grep",
    "filesystem/list_dir",
    "filesystem/tree",
    "filesystem/file_info",
    "system/process_list",
    "git/status",
    "git/diff",
    "git/log",
    "data/db_read",
    "data/storage_read",
    "data/vault_read",
    "data/entity_query",
    "data/web_scrape",
    "data/html_to_markdown",
})


@dataclass(frozen=True)
class AutonomyConfig:
    level: str  # assisted | copilot | autopilot | self_driving
    max_tool_rounds: int
    confirm_writes: bool
    confirm_reads: bool
    auto_checkpoint: bool

    def needs_confirmation(self, tool_type: str) -> bool:
        """Check if a tool call requires user confirmation at this autonomy level."""
        if tool_type in WRITE_TOOLS:
            return self.confirm_writes
        if tool_type in READ_TOOLS:
            return self.confirm_reads
        # Unknown tools: confirm at assisted/copilot, skip at autopilot+
        return self.level in ("assisted", "copilot")


AUTONOMY_CONFIGS: dict[str, AutonomyConfig] = {
    "assisted": AutonomyConfig(
        level="assisted",
        max_tool_rounds=1,
        confirm_writes=True,
        confirm_reads=False,
        auto_checkpoint=True,
    ),
    "copilot": AutonomyConfig(
        level="copilot",
        max_tool_rounds=25,
        confirm_writes=False,
        confirm_reads=False,
        auto_checkpoint=True,
    ),
    "autopilot": AutonomyConfig(
        level="autopilot",
        max_tool_rounds=50,
        confirm_writes=False,
        confirm_reads=False,
        auto_checkpoint=True,
    ),
    "self_driving": AutonomyConfig(
        level="self_driving",
        max_tool_rounds=100,
        confirm_writes=False,
        confirm_reads=False,
        auto_checkpoint=False,  # too many rounds, checkpoint manually
    ),
}


def get_autonomy_config(level: str) -> AutonomyConfig:
    """Get autonomy config by level name. Defaults to copilot."""
    return AUTONOMY_CONFIGS.get(level, AUTONOMY_CONFIGS["copilot"])


def ask_user_confirmation(tool_type: str, args: dict[str, Any]) -> bool:
    """Prompt user to confirm a tool call. Returns True if approved."""
    import sys

    args_preview = ", ".join(f"{k}={str(v)[:40]}" for k, v in list(args.items())[:3])
    try:
        resp = input(f"  \033[33m? Confirm {tool_type}({args_preview})? [Y/n]: \033[0m").strip().lower()
    except (KeyboardInterrupt, EOFError):
        print()
        return False
    return resp in ("", "y", "yes", "si", "s")
