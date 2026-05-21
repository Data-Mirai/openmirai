"""Tests for autonomy_levels — confirmation logic per level."""

from __future__ import annotations

import pytest

from datamirai_engine.cli.autonomy_levels import (
    AUTONOMY_CONFIGS,
    AutonomyConfig,
    READ_TOOLS,
    WRITE_TOOLS,
    get_autonomy_config,
)


class TestAutonomyConfig:
    def test_all_levels_defined(self):
        assert "assisted" in AUTONOMY_CONFIGS
        assert "copilot" in AUTONOMY_CONFIGS
        assert "autopilot" in AUTONOMY_CONFIGS
        assert "self_driving" in AUTONOMY_CONFIGS

    def test_get_default_is_copilot(self):
        config = get_autonomy_config("nonexistent")
        assert config.level == "copilot"

    def test_assisted_rounds(self):
        c = get_autonomy_config("assisted")
        assert c.max_tool_rounds == 1

    def test_self_driving_rounds(self):
        c = get_autonomy_config("self_driving")
        assert c.max_tool_rounds == 100


class TestConfirmationLogic:
    def test_assisted_confirms_writes(self):
        c = get_autonomy_config("assisted")
        assert c.needs_confirmation("filesystem/write_file") is True
        assert c.needs_confirmation("filesystem/edit_file") is True
        assert c.needs_confirmation("system/bash") is True
        assert c.needs_confirmation("git/commit") is True

    def test_assisted_does_not_confirm_reads(self):
        c = get_autonomy_config("assisted")
        assert c.needs_confirmation("filesystem/read_file") is False
        assert c.needs_confirmation("filesystem/glob") is False
        assert c.needs_confirmation("git/status") is False

    def test_copilot_no_confirm_on_anything(self):
        c = get_autonomy_config("copilot")
        assert c.needs_confirmation("filesystem/write_file") is False
        assert c.needs_confirmation("filesystem/read_file") is False
        assert c.needs_confirmation("system/bash") is False

    def test_autopilot_no_confirm(self):
        c = get_autonomy_config("autopilot")
        assert c.needs_confirmation("filesystem/delete") is False
        assert c.needs_confirmation("git/commit") is False

    def test_self_driving_no_confirm(self):
        c = get_autonomy_config("self_driving")
        assert c.needs_confirmation("filesystem/write_file") is False

    def test_unknown_tool_assisted_confirms(self):
        c = get_autonomy_config("assisted")
        assert c.needs_confirmation("unknown/weird_tool") is True

    def test_unknown_tool_autopilot_skips(self):
        c = get_autonomy_config("autopilot")
        assert c.needs_confirmation("unknown/weird_tool") is False


class TestToolClassification:
    def test_write_tools_not_empty(self):
        assert len(WRITE_TOOLS) > 0

    def test_read_tools_not_empty(self):
        assert len(READ_TOOLS) > 0

    def test_no_overlap(self):
        overlap = WRITE_TOOLS & READ_TOOLS
        assert len(overlap) == 0, f"Overlap: {overlap}"

    def test_bash_is_write(self):
        assert "system/bash" in WRITE_TOOLS

    def test_read_file_is_read(self):
        assert "filesystem/read_file" in READ_TOOLS

    def test_git_commit_is_write(self):
        assert "git/commit" in WRITE_TOOLS

    def test_git_status_is_read(self):
        assert "git/status" in READ_TOOLS
