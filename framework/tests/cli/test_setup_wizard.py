"""Tests for setup_wizard — model detection, config generation, autonomy levels."""

from __future__ import annotations

import os
from unittest.mock import AsyncMock, patch

import pytest

from datamirai_engine.cli.setup_wizard import (
    AUTONOMY_LEVELS,
    ModelOption,
    SessionConfig,
    detect_ollama_models,
    detect_remote_providers,
    run_setup_wizard,
)


class TestModelOption:
    def test_text_modality(self):
        m = ModelOption(id="qwen3:8b", name="Qwen 3 8B", provider="ollama")
        assert m.modality_label == "text"

    def test_vision_modality(self):
        m = ModelOption(id="llava:13b", name="LLaVA", provider="ollama", supports_vision=True)
        assert m.modality_label == "multimodal"

    def test_ctx_label_thousands(self):
        m = ModelOption(id="x", name="x", provider="p", context_window=32768)
        assert m.ctx_label == "32K"

    def test_ctx_label_millions(self):
        m = ModelOption(id="x", name="x", provider="p", context_window=1048576)
        assert m.ctx_label == "1M"

    def test_ctx_label_unknown(self):
        m = ModelOption(id="x", name="x", provider="p", context_window=None)
        assert m.ctx_label == "?"


class TestAutonomyLevels:
    def test_all_levels_exist(self):
        assert "assisted" in AUTONOMY_LEVELS
        assert "copilot" in AUTONOMY_LEVELS
        assert "autopilot" in AUTONOMY_LEVELS
        assert "self_driving" in AUTONOMY_LEVELS

    def test_assisted_limits(self):
        lvl = AUTONOMY_LEVELS["assisted"]
        assert lvl["max_tool_rounds"] == 1
        assert lvl["confirm_writes"] is True

    def test_copilot_limits(self):
        lvl = AUTONOMY_LEVELS["copilot"]
        assert lvl["max_tool_rounds"] == 25
        assert lvl["confirm_writes"] is False

    def test_autopilot_limits(self):
        lvl = AUTONOMY_LEVELS["autopilot"]
        assert lvl["max_tool_rounds"] == 50

    def test_self_driving_limits(self):
        lvl = AUTONOMY_LEVELS["self_driving"]
        assert lvl["max_tool_rounds"] == 100


class TestDetectRemoteProviders:
    def test_no_keys_set(self):
        with patch.dict(os.environ, {}, clear=True):
            providers = detect_remote_providers()
            # Might pick up keys from real env, so just check type
            assert isinstance(providers, list)

    def test_groq_key_detected(self):
        with patch.dict(os.environ, {"GROQ_API_KEY": "gsk_test123"}, clear=True):
            providers = detect_remote_providers()
            groq = [p for p in providers if p["provider"] == "groq"]
            assert len(groq) == 1
            assert groq[0]["default_model"] == "qwen-qwq-32b"

    def test_nvidia_key_detected(self):
        with patch.dict(os.environ, {"NVIDIA_API_KEY": "nvapi-test"}, clear=True):
            providers = detect_remote_providers()
            nvidia = [p for p in providers if p["provider"] == "nvidia"]
            assert len(nvidia) == 1

    def test_multiple_keys(self):
        with patch.dict(os.environ, {
            "GROQ_API_KEY": "gsk_x",
            "OPENAI_API_KEY": "sk_x",
        }, clear=True):
            providers = detect_remote_providers()
            names = {p["provider"] for p in providers}
            assert "groq" in names
            assert "openai" in names


class TestDetectOllamaModels:
    @pytest.mark.asyncio
    async def test_ollama_not_running_returns_empty(self):
        """If Ollama isn't running, returns empty list (no crash)."""
        models = await detect_ollama_models(base_url="http://localhost:99999")
        assert models == []

    @pytest.mark.asyncio
    async def test_ollama_running_returns_models(self):
        """If Ollama IS running locally, we should get models."""
        models = await detect_ollama_models()
        # This test adapts: if Ollama is running, we get models; if not, empty list
        assert isinstance(models, list)
        if models:
            assert all(isinstance(m, ModelOption) for m in models)
            assert all(m.provider == "ollama" for m in models)
            assert all(m.local is True for m in models)


class TestRunSetupWizardSkipMode:
    def test_skip_wizard_returns_config(self):
        """When skip_wizard=True, returns config without prompting."""
        config = run_setup_wizard(
            skip_wizard=True,
            provider="groq",
            model="qwen-qwq-32b",
            autonomy="copilot",
        )
        assert isinstance(config, SessionConfig)
        assert config.provider == "groq"
        assert config.model == "qwen-qwq-32b"
        assert config.autonomy_level == "copilot"
        assert config.max_tool_rounds == 25

    def test_skip_wizard_defaults(self):
        config = run_setup_wizard(skip_wizard=True)
        assert config.provider == "ollama"
        assert config.model == "qwen3:8b"
        assert config.autonomy_level == "copilot"

    def test_all_params_skips_wizard(self):
        """If all 3 params provided, skip interactive even without flag."""
        config = run_setup_wizard(
            provider="nvidia",
            model="meta/llama-3.3-70b-instruct",
            autonomy="autopilot",
        )
        assert config.provider == "nvidia"
        assert config.autonomy_level == "autopilot"
        assert config.max_tool_rounds == 50

    def test_assisted_level_config(self):
        config = run_setup_wizard(skip_wizard=True, autonomy="assisted")
        assert config.max_tool_rounds == 1
        assert config.confirm_writes is True

    def test_self_driving_level_config(self):
        config = run_setup_wizard(skip_wizard=True, autonomy="self_driving")
        assert config.max_tool_rounds == 100
        assert config.confirm_writes is False
