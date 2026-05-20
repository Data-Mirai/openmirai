"""Tests for CLI."""

from __future__ import annotations

from datamirai_engine.cli import main


class TestCLI:
    def test_version(self, capsys):
        main(["version"])
        captured = capsys.readouterr()
        assert "datamirai-engine v" in captured.out

    def test_no_command_exits(self):
        import pytest
        with pytest.raises(SystemExit):
            main([])
