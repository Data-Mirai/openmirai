"""Tests for stealth utilities."""

from datamirai_engine.tools.builtin.data.stealth import (
    USER_AGENT_POOL,
    VIEWPORT_POOL,
    apply_jitter,
    get_random_user_agent,
    get_random_viewport,
    get_stealth_headers,
    stealth_context_options,
)


class TestUserAgentPool:
    def test_has_10_plus_agents(self):
        assert len(USER_AGENT_POOL) >= 10

    def test_all_agents_are_strings(self):
        for ua in USER_AGENT_POOL:
            assert isinstance(ua, str)
            assert len(ua) > 50

    def test_get_random_returns_from_pool(self):
        for _ in range(20):
            ua = get_random_user_agent()
            assert ua in USER_AGENT_POOL


class TestStealthHeaders:
    def test_contains_required_keys(self):
        headers = get_stealth_headers()
        assert "User-Agent" in headers
        assert "Accept" in headers
        assert "Accept-Language" in headers
        assert "Sec-Fetch-Dest" in headers
        assert "Sec-Fetch-Mode" in headers

    def test_user_agent_from_pool(self):
        headers = get_stealth_headers()
        assert headers["User-Agent"] in USER_AGENT_POOL


class TestViewport:
    def test_valid_dimensions(self):
        for vp in VIEWPORT_POOL:
            assert vp["width"] > 0
            assert vp["height"] > 0

    def test_random_returns_valid(self):
        vp = get_random_viewport()
        assert "width" in vp
        assert "height" in vp
        assert vp["width"] >= 1280


class TestJitter:
    def test_jitter_within_bounds(self):
        """REGLA-366: Jitter must be within ±30%."""
        delay = 1.0
        results = [apply_jitter(delay) for _ in range(200)]
        for r in results:
            assert 0.7 * delay <= r <= 1.3 * delay

    def test_jitter_varies(self):
        results = {apply_jitter(1.0) for _ in range(50)}
        assert len(results) > 10  # should have variety


class TestContextOptions:
    def test_has_required_fields(self):
        opts = stealth_context_options()
        assert "user_agent" in opts
        assert "viewport" in opts
        assert opts["viewport"]["width"] > 0
