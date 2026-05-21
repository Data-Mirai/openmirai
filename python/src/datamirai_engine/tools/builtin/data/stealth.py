"""Stealth utilities for anti-bot detection evasion.

Provides realistic user agents, headers, viewports, timing jitter,
and SessionFingerprint for consistent identity per session.

REGLA-366: Stealth delays use ±30% jitter.
REGLA-380: SessionFingerprint is immutable once generated.
REGLA-381: Internal coherence: UA + Sec-CH-UA + Platform + Viewport must be consistent.
REGLA-382: One fingerprint per session (managed) or per cycle/run (live).
"""

from __future__ import annotations

import random
import re
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any


# 12 modern user agents from real browsers (2024-2026)
USER_AGENT_POOL = [
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:126.0) Gecko/20100101 Firefox/126.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:126.0) Gecko/20100101 Firefox/126.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:126.0) Gecko/20100101 Firefox/126.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36 Edg/125.0.0.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36 OPR/111.0.0.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36",
]

# Common viewport sizes
VIEWPORT_POOL = [
    {"width": 1920, "height": 1080},
    {"width": 1366, "height": 768},
    {"width": 1280, "height": 800},
    {"width": 1440, "height": 900},
    {"width": 1536, "height": 864},
]

# Platform ↔ UA coherence map
_PLATFORM_MAP: dict[str, str] = {
    "Macintosh": "macOS",
    "Windows NT": "Windows",
    "X11; Linux": "Linux",
}

# Locale pool — realistic variety
_LOCALE_POOL = ["es-ES", "en-US", "es-MX", "en-GB"]

# Timezone pool
_TIMEZONE_POOL = [
    "America/Bogota",
    "America/Mexico_City",
    "America/New_York",
    "America/Los_Angeles",
    "Europe/Madrid",
]


def _detect_browser(ua: str) -> str:
    """Detect browser type from user agent string."""
    if "Edg/" in ua:
        return "edge"
    if "OPR/" in ua:
        return "opera"
    if "Firefox/" in ua:
        return "firefox"
    if "Safari/" in ua and "Chrome/" not in ua:
        return "safari"
    if "Chrome/" in ua:
        return "chrome"
    return "unknown"


def _detect_platform(ua: str) -> str:
    """Detect platform from user agent string."""
    for pattern, platform in _PLATFORM_MAP.items():
        if pattern in ua:
            return platform
    return "Windows"


def _extract_chrome_version(ua: str) -> str:
    """Extract Chrome major version from UA string."""
    match = re.search(r"Chrome/(\d+)", ua)
    return match.group(1) if match else "125"


@dataclass(frozen=True)
class SessionFingerprint:
    """Immutable identity for a scraping session.

    REGLA-380: Frozen once generated — never modified during execution.
    REGLA-381: All fields are internally coherent (UA ↔ platform ↔ headers).
    REGLA-382: Generated once per session (managed) or per cycle/run (live).
    """

    user_agent: str
    headers: dict[str, str] = field(hash=False)
    viewport: dict[str, int] = field(hash=False)
    platform: str = ""
    locale: str = "es-ES"
    timezone: str = "America/Bogota"
    sec_ch_ua: str = ""
    browser: str = ""
    proxy_host: str | None = None
    created_at: str = ""

    @classmethod
    def generate(cls, *, proxy_host: str | None = None) -> SessionFingerprint:
        """Generate a coherent fingerprint with internally consistent fields."""
        ua = random.choice(USER_AGENT_POOL)
        browser = _detect_browser(ua)
        platform = _detect_platform(ua)
        viewport = dict(random.choice(VIEWPORT_POOL))
        locale = random.choice(_LOCALE_POOL)
        tz = random.choice(_TIMEZONE_POOL)

        # Build coherent headers
        headers: dict[str, str] = {
            "User-Agent": ua,
            "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            "Accept-Language": f"{locale},{locale.split('-')[0]};q=0.9,en-US;q=0.8,en;q=0.7",
            "Accept-Encoding": "gzip, deflate, br",
            "Connection": "keep-alive",
            "Upgrade-Insecure-Requests": "1",
            "Sec-Fetch-Dest": "document",
            "Sec-Fetch-Mode": "navigate",
            "Sec-Fetch-Site": "none",
            "Sec-Fetch-User": "?1",
        }

        # Sec-CH-UA only for Chromium-based browsers (REGLA-381)
        sec_ch_ua = ""
        if browser in ("chrome", "edge", "opera"):
            version = _extract_chrome_version(ua)
            platform_quoted = f'"{platform}"'
            if browser == "chrome":
                sec_ch_ua = f'"Chromium";v="{version}", "Google Chrome";v="{version}", "Not.A/Brand";v="24"'
            elif browser == "edge":
                sec_ch_ua = f'"Chromium";v="{version}", "Microsoft Edge";v="{version}", "Not.A/Brand";v="24"'
            elif browser == "opera":
                sec_ch_ua = f'"Chromium";v="{version}", "Opera";v="111", "Not.A/Brand";v="24"'
            headers["Sec-CH-UA"] = sec_ch_ua
            headers["Sec-CH-UA-Mobile"] = "?0"
            headers["Sec-CH-UA-Platform"] = platform_quoted

        now = datetime.now(timezone.utc).isoformat()

        return cls(
            user_agent=ua,
            headers=headers,
            viewport=viewport,
            platform=platform,
            locale=locale,
            timezone=tz,
            sec_ch_ua=sec_ch_ua,
            browser=browser,
            proxy_host=proxy_host,
            created_at=now,
        )

    def to_playwright_context(self) -> dict[str, Any]:
        """Convert to Playwright browser context options."""
        return {
            "user_agent": self.user_agent,
            "viewport": dict(self.viewport),
            "locale": self.locale,
            "timezone_id": self.timezone,
            "color_scheme": "light",
            "java_script_enabled": True,
            "bypass_csp": True,
        }

    def to_dict(self) -> dict[str, Any]:
        """Serialize for manifest/logging. REGLA-399: no proxy credentials."""
        return {
            "user_agent": self.user_agent,
            "platform": self.platform,
            "browser": self.browser,
            "viewport": dict(self.viewport),
            "locale": self.locale,
            "timezone": self.timezone,
            "proxy_host": self.proxy_host,
            "created_at": self.created_at,
        }


def get_random_user_agent() -> str:
    """Get a random user agent from the pool."""
    return random.choice(USER_AGENT_POOL)


def get_stealth_headers() -> dict[str, str]:
    """Get realistic browser headers for anti-bot evasion."""
    return dict(SessionFingerprint.generate().headers)


def get_random_viewport() -> dict[str, int]:
    """Get a random viewport size."""
    return dict(random.choice(VIEWPORT_POOL))


def apply_jitter(delay: float) -> float:
    """Apply ±30% random jitter to a delay value. REGLA-366."""
    jitter_factor = 0.7 + random.random() * 0.6  # range: [0.7, 1.3]
    return delay * jitter_factor


def stealth_context_options() -> dict:
    """Get Playwright browser context options for stealth mode."""
    return SessionFingerprint.generate().to_playwright_context()


def select_proxy(proxy_url: str, proxy_list: str, session_id: str | None = None) -> str | None:
    """Select a proxy for the session. REGLA-385/397: one proxy per session.

    Uses session_id as seed for deterministic selection from proxy_list.
    """
    if proxy_url:
        return proxy_url.strip()

    proxies = [p.strip() for p in proxy_list.splitlines() if p.strip()]
    if not proxies:
        return None

    if session_id:
        idx = hash(session_id) % len(proxies)
    else:
        idx = random.randrange(len(proxies))
    return proxies[idx]


def proxy_host_safe(proxy_url: str) -> str:
    """Extract host:port from proxy URL, stripping credentials. REGLA-399."""
    from urllib.parse import urlparse
    parsed = urlparse(proxy_url)
    host = parsed.hostname or ""
    port = parsed.port
    return f"{host}:{port}" if port else host
