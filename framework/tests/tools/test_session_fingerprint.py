"""Tests for SessionFingerprint — FEAT-023.

REGLA-380: Immutable once generated.
REGLA-381: Internal coherence (UA ↔ platform ↔ headers).
REGLA-382: One per session/cycle.
REGLA-385/397: Proxy one per session.
REGLA-399: No proxy credentials in serialization.
"""

import pytest
from datamirai_engine.tools.builtin.data.stealth import (
    SessionFingerprint,
    USER_AGENT_POOL,
    select_proxy,
    proxy_host_safe,
)


class TestSessionFingerprintGenerate:
    def test_returns_frozen_dataclass(self):
        fp = SessionFingerprint.generate()
        with pytest.raises(AttributeError):
            fp.user_agent = "changed"  # type: ignore[misc]

    def test_user_agent_from_pool(self):
        for _ in range(20):
            fp = SessionFingerprint.generate()
            assert fp.user_agent in USER_AGENT_POOL

    def test_headers_include_user_agent(self):
        fp = SessionFingerprint.generate()
        assert fp.headers["User-Agent"] == fp.user_agent

    def test_platform_coherent_with_ua(self):
        """REGLA-381: Platform must match the UA string."""
        for _ in range(50):
            fp = SessionFingerprint.generate()
            if "Macintosh" in fp.user_agent:
                assert fp.platform == "macOS"
            elif "Windows" in fp.user_agent:
                assert fp.platform == "Windows"
            elif "Linux" in fp.user_agent:
                assert fp.platform == "Linux"

    def test_sec_ch_ua_only_for_chromium(self):
        """REGLA-381: Firefox/Safari should NOT have Sec-CH-UA."""
        for _ in range(100):
            fp = SessionFingerprint.generate()
            if fp.browser == "firefox" or fp.browser == "safari":
                assert "Sec-CH-UA" not in fp.headers
                assert fp.sec_ch_ua == ""
            elif fp.browser in ("chrome", "edge", "opera"):
                assert "Sec-CH-UA" in fp.headers
                assert fp.sec_ch_ua != ""

    def test_sec_ch_ua_platform_matches(self):
        """REGLA-381: Sec-CH-UA-Platform must match detected platform."""
        for _ in range(100):
            fp = SessionFingerprint.generate()
            if "Sec-CH-UA-Platform" in fp.headers:
                platform_header = fp.headers["Sec-CH-UA-Platform"]
                assert fp.platform in platform_header

    def test_viewport_has_dimensions(self):
        fp = SessionFingerprint.generate()
        assert fp.viewport["width"] > 0
        assert fp.viewport["height"] > 0

    def test_created_at_populated(self):
        fp = SessionFingerprint.generate()
        assert fp.created_at != ""
        assert "T" in fp.created_at

    def test_browser_detected(self):
        fp = SessionFingerprint.generate()
        assert fp.browser in ("chrome", "firefox", "safari", "edge", "opera")

    def test_locale_populated(self):
        fp = SessionFingerprint.generate()
        assert "-" in fp.locale

    def test_proxy_host_stored(self):
        fp = SessionFingerprint.generate(proxy_host="proxy.example.com:8080")
        assert fp.proxy_host == "proxy.example.com:8080"

    def test_default_no_proxy(self):
        fp = SessionFingerprint.generate()
        assert fp.proxy_host is None


class TestSessionFingerprintPlaywright:
    def test_to_playwright_context(self):
        fp = SessionFingerprint.generate()
        opts = fp.to_playwright_context()
        assert opts["user_agent"] == fp.user_agent
        assert opts["viewport"]["width"] == fp.viewport["width"]
        assert opts["locale"] == fp.locale
        assert opts["timezone_id"] == fp.timezone


class TestSessionFingerprintSerialization:
    def test_to_dict_has_fields(self):
        fp = SessionFingerprint.generate()
        d = fp.to_dict()
        assert "user_agent" in d
        assert "platform" in d
        assert "browser" in d
        assert "viewport" in d
        assert "locale" in d
        assert "created_at" in d

    def test_to_dict_no_headers(self):
        """Headers contain secrets — should not be in public serialization."""
        fp = SessionFingerprint.generate()
        d = fp.to_dict()
        assert "headers" not in d

    def test_to_dict_no_credentials(self):
        """REGLA-399: Proxy credentials never in serialization."""
        fp = SessionFingerprint.generate(proxy_host="proxy.example.com:8080")
        d = fp.to_dict()
        assert d["proxy_host"] == "proxy.example.com:8080"
        assert "user" not in str(d["proxy_host"])
        assert "pass" not in str(d["proxy_host"])


class TestSelectProxy:
    def test_proxy_url_takes_precedence(self):
        result = select_proxy("http://my-proxy:8080", "http://other:9090")
        assert result == "http://my-proxy:8080"

    def test_proxy_list_selects_one(self):
        proxy_list = "http://p1:8080\nhttp://p2:8080\nhttp://p3:8080"
        result = select_proxy("", proxy_list, session_id="test-session")
        assert result in ["http://p1:8080", "http://p2:8080", "http://p3:8080"]

    def test_proxy_list_deterministic_with_session_id(self):
        """REGLA-385: Same session_id always selects same proxy."""
        proxy_list = "http://p1:8080\nhttp://p2:8080\nhttp://p3:8080"
        results = {select_proxy("", proxy_list, session_id="fixed-id") for _ in range(20)}
        assert len(results) == 1

    def test_no_proxy_returns_none(self):
        assert select_proxy("", "") is None
        assert select_proxy("", "\n\n") is None

    def test_strips_whitespace(self):
        result = select_proxy("  http://proxy:8080  ", "")
        assert result == "http://proxy:8080"


class TestProxyHostSafe:
    def test_strips_credentials(self):
        """REGLA-399."""
        result = proxy_host_safe("http://user:pass@proxy.example.com:8080")
        assert result == "proxy.example.com:8080"
        assert "user" not in result
        assert "pass" not in result

    def test_simple_proxy(self):
        result = proxy_host_safe("http://proxy.example.com:8080")
        assert result == "proxy.example.com:8080"

    def test_socks_proxy(self):
        result = proxy_host_safe("socks5://user:pass@socks.example.com:1080")
        assert result == "socks.example.com:1080"
