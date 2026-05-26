"""Web Scrape tool v2.1 — stealth-first web scraping with two modes.

v2.1.0 (FEAT-023) over v2.0.0:
- Stealth-first: SessionFingerprint for all requests — REGLA-383
- Two modes: windowless (HTTP stealth) and tunnel_vision (browser + screenshot-loop)
- Referer chain: simulates human navigation — REGLA-386
- Domain backoff: cumulative delay on 429s — REGLA-384
- Proxy support: one proxy per session — REGLA-385/397

Preserved from v2.0.0:
- Markdown as default output format — REGLA-353
- URL cache with configurable TTL — REGLA-356
- Retry with exponential backoff — REGLA-357
- Per-domain rate limiting — REGLA-359
- Structured extraction via LLM with JSON schema — REGLA-355
- Auto-vault integration — REGLA-358
- HTML-to-Markdown converter for clean output — REGLA-354
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import logging
import random
import re
from datetime import datetime, timezone
from typing import Any
from urllib.parse import quote_plus, unquote, urlparse, urlencode, parse_qs

from datamirai_engine.tools.base import BaseTool, ToolInput, ToolOutput, ToolSpec, ConfigField
from datamirai_engine.core.context import ExecutionContext
from datamirai_engine.tools.builtin.data.stealth import (
    SessionFingerprint,
    apply_jitter,
    select_proxy,
    proxy_host_safe,
)

logger = logging.getLogger(__name__)

# Domains to skip when extracting links from search result pages
_SKIP_DOMAINS = frozenset({
    "google.com", "google.co", "gstatic.com", "googleapis.com",
    "bing.com", "microsoft.com", "msn.com", "live.com",
    "duckduckgo.com", "brave.com",
    "schema.org", "w3.org",
    "youtube.com", "maps.google.com",
    "facebook.com", "instagram.com",
    "apple.com", "play.google.com",
})


class WebScrapeTool(BaseTool):
    spec = ToolSpec(
        tool_type="data/web_scrape",
        version="2.0.0",
        display_name="Web Scrape",
        description="Fetches multiple web pages and extracts their content for research",
        category="data",
        icon="globe",
        intents=[
            "buscar informacion en internet sobre un tema especifico",
            "investigar noticias, eventos, tendencias, productos, competidores",
            "extraer contenido de paginas web especificas (URLs directas)",
            "recopilar datos de multiples fuentes para analisis posterior",
            "busqueda filtrada por fecha: noticias recientes, ultima semana, ultimo mes",
        ],
        inputs=[
            ToolInput(name="urls", type="array", required=False,
                       description="Override URLs at runtime (optional, falls back to config)"),
            ToolInput(name="query", type="string", required=False,
                       description="Search query — auto-generates search engine URLs if no urls provided"),
            ToolInput(name="headers", type="object", required=False,
                       description="Optional HTTP headers"),
        ],
        outputs=[
            ToolOutput(name="results", type="array"),
            ToolOutput(name="total", type="number"),
            ToolOutput(name="successful", type="number"),
            ToolOutput(name="vault_paths", type="array"),
        ],
        config=[
            ConfigField(name="query", type="string", default="",
                        description="Search queries (one per line) — searches on configured engines"),
            ConfigField(name="urls", type="string", default="",
                        description="URLs to scrape, one per line"),
            ConfigField(name="mode", type="select", default="markdown",
                        options=["text", "html", "markdown"],
                        description="Formato de salida: markdown (default v2), text, html"),
            ConfigField(name="timeout", type="number", default=30),
            ConfigField(name="max_content_length", type="number", default=15000,
                        description="Max characters per page (truncates to save LLM context)"),
            ConfigField(name="search_engines", type="string", default="google,bing,duckduckgo",
                        description="Search engines to use when query is provided"),
            ConfigField(name="date_range", type="select", default="any",
                        options=["any", "day", "week", "month"],
                        description="Filter search results by recency"),
            ConfigField(name="max_results_per_query", type="number", default=3,
                        description="Max article links to follow per search engine result"),
            # v2 features
            ConfigField(name="cache_ttl", type="number", default=3600,
                        description="Segundos que un resultado cacheado es valido. 0 = sin cache"),
            ConfigField(name="max_retries", type="number", default=3,
                        description="Reintentos por URL con exponential backoff"),
            ConfigField(name="request_delay", type="number", default=0.5,
                        description="Segundos de espera entre requests al mismo dominio"),
            ConfigField(name="extraction_schema", type="string", default="",
                        description="JSON schema para extraccion estructurada via LLM"),
            ConfigField(name="auto_vault", type="boolean", default=False,
                        description="Guardar resultados como notas en el Knowledge Vault"),
            ConfigField(name="vault_folder", type="string", default="vault/scrapes",
                        description="Carpeta destino en el vault"),
            ConfigField(name="vault_tags", type="string", default="",
                        description="Tags para las notas del vault (separados por coma)"),
            # FEAT-023: Stealth + modes
            ConfigField(name="scrape_mode", type="select", default="windowless",
                        options=["windowless", "tunnel_vision"],
                        description="windowless=HTTP stealth rapido, tunnel_vision=browser visual con screenshot-loop"),
            ConfigField(name="proxy_url", type="string", default="",
                        description="Proxy HTTP/SOCKS5 (ej: http://user:pass@proxy:8080)"),
            ConfigField(name="proxy_list", type="string", default="",
                        description="Lista de proxies, uno por linea. Se elige uno al azar por sesion"),
            ConfigField(name="agent_objective", type="string", default="",
                        description="Objetivo en lenguaje natural para modo Tunnel Vision"),
            ConfigField(name="agent_model", type="string", default="claude-sonnet",
                        description="Modelo LLM con vision para screenshot-loop"),
            ConfigField(name="max_steps", type="number", default=10,
                        description="Max pasos del screenshot-loop (hard limit: 25)"),
        ],
    )

    async def execute(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        scrape_mode = config.get("scrape_mode", "windowless")

        # --- Tunnel Vision mode → delegate to browser_agent ---
        if scrape_mode == "tunnel_vision":
            return await self._execute_tunnel_vision(inputs, config, context)

        # --- Windowless mode (default) ---
        return await self._execute_windowless(inputs, config, context)

    async def _execute_tunnel_vision(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        """Tunnel Vision: browser screenshot-loop with live streaming."""
        from datamirai_engine.tools.builtin.data.browser_agent import agent_scrape

        raw_urls_input = inputs.get("urls") or config.get("urls", "")
        if isinstance(raw_urls_input, str):
            urls = [u.strip() for u in raw_urls_input.splitlines() if u.strip()]
        elif isinstance(raw_urls_input, list):
            urls = list(raw_urls_input)
        else:
            urls = []

        if not urls:
            raise ValueError("Tunnel Vision mode requires at least one URL")

        objective = config.get("agent_objective", "")
        if not objective:
            raise ValueError("Tunnel Vision mode requires agent_objective configured")

        model = config.get("agent_model", "claude-sonnet")
        max_steps = int(config.get("max_steps", 10))

        # Get fingerprint from context (REGLA-383/387)
        fingerprint = getattr(context, "fingerprint", None)
        if not fingerprint:
            fingerprint = SessionFingerprint.generate()

        # Select proxy (REGLA-385/397)
        proxy_url = config.get("proxy_url", "")
        proxy_list = config.get("proxy_list", "")
        session_id = getattr(context, "session_id", None)
        proxy = select_proxy(proxy_url, proxy_list, session_id)

        results: list[dict[str, Any]] = []
        for url in urls:
            try:
                result = await agent_scrape(
                    url, objective, context,
                    max_steps=max_steps,
                    model=model,
                    stealth=True,
                    fingerprint=fingerprint,
                    proxy=proxy,
                    save_screenshots=True,
                )
                results.append({
                    "url": url,
                    "title": "",
                    "content": "\n\n".join(result.get("data", [])),
                    "status_code": 200,
                    "success": True,
                    "mode": "tunnel_vision",
                    "steps": result.get("steps", 0),
                })
            except Exception as exc:
                results.append({
                    "url": url,
                    "title": "",
                    "content": str(exc),
                    "status_code": 0,
                    "success": False,
                    "mode": "tunnel_vision",
                })

        successful = sum(1 for r in results if r["success"])
        if successful == 0 and results:
            errors = "; ".join(f"{r['url'][:50]}: {r['content'][:80]}" for r in results[:3])
            raise ValueError(f"All {len(results)} URLs failed. Errors: {errors}")

        return {"results": results, "total": len(results), "successful": successful, "vault_paths": []}

    async def _execute_windowless(
        self, inputs: dict[str, Any], config: dict[str, Any], context: ExecutionContext | None
    ) -> dict[str, Any]:
        """Windowless: stealth HTTP scraping. REGLA-383."""
        import httpx

        # --- Get session fingerprint (REGLA-383: always stealth) ---
        fingerprint = getattr(context, "fingerprint", None)
        if not fingerprint:
            fingerprint = SessionFingerprint.generate()

        # --- Select proxy (REGLA-385/397: one per session) ---
        proxy_url = config.get("proxy_url", "")
        proxy_list = config.get("proxy_list", "")
        session_id = getattr(context, "session_id", None)
        proxy = select_proxy(proxy_url, proxy_list, session_id)

        # --- Build direct URL list ---
        raw_urls_input = inputs.get("urls") or config.get("urls", "")
        if isinstance(raw_urls_input, str):
            direct_urls = [u.strip() for u in raw_urls_input.splitlines() if u.strip()]
        elif isinstance(raw_urls_input, list):
            direct_urls = list(raw_urls_input)
        else:
            direct_urls = []

        # --- Build search queries ---
        query_input = inputs.get("query") or config.get("query", "")
        queries = [q.strip() for q in query_input.splitlines() if q.strip()] if query_input else []

        date_range = config.get("date_range", "any")
        engines = config.get("search_engines", "google,bing,duckduckgo")
        max_per_query = int(config.get("max_results_per_query", 3))

        search_urls: list[tuple[str, str]] = []
        for q in queries:
            for url in _build_search_urls(q, engines, date_range):
                search_urls.append((url, q))

        if not direct_urls and not search_urls:
            raise ValueError("No URLs or search query provided")

        headers_extra = inputs.get("headers") or {}
        mode = config.get("mode", "text")
        timeout = int(config.get("timeout", 30))
        max_len = int(config.get("max_content_length", 15000))

        max_retries = int(config.get("max_retries", 3))
        request_delay = float(config.get("request_delay", 0.5))
        cache_ttl = int(config.get("cache_ttl", 3600))
        extraction_schema = config.get("extraction_schema", "")
        auto_vault = config.get("auto_vault", False)
        if isinstance(auto_vault, str):
            auto_vault = auto_vault.lower() in ("true", "1", "yes")

        results: list[dict[str, Any]] = []

        # Domain backoff tracker — REGLA-384: cumulative per session
        domain_delays: dict[str, float] = {}

        client_kwargs: dict[str, Any] = {"follow_redirects": True, "timeout": timeout}
        if proxy:
            client_kwargs["proxy"] = proxy

        async with httpx.AsyncClient(**client_kwargs) as client:

            # --- Phase 1: Fetch SERPs ---
            if search_urls:
                serp_results = await _fetch_urls_parallel(
                    client, [u for u, _ in search_urls], headers_extra,
                    fingerprint=fingerprint, mode="html",
                    max_retries=max_retries, request_delay=request_delay,
                    domain_delays=domain_delays,
                )

                # --- Phase 2: Extract article links ---
                article_urls: list[str] = []
                seen: set[str] = set()
                max_total = max_per_query * max(len(queries), 1)

                for serp in serp_results:
                    if not serp["success"]:
                        continue
                    engine = _detect_engine(serp["url"])
                    links = _extract_serp_links(serp["content"], engine)
                    for link in links:
                        if link not in seen and len(article_urls) < max_total:
                            seen.add(link)
                            article_urls.append(link)

                # --- Phase 3: Fetch articles ---
                if article_urls:
                    article_results = await _fetch_urls_parallel(
                        client, article_urls, headers_extra,
                        fingerprint=fingerprint,
                        mode=mode, max_len=max_len,
                        max_retries=max_retries, request_delay=request_delay,
                        cache_ttl=cache_ttl, context=context,
                        domain_delays=domain_delays,
                    )
                    results.extend(article_results)
                else:
                    for serp in serp_results:
                        if serp["success"]:
                            text = _html_to_text(serp["content"])
                            if max_len and len(text) > max_len:
                                text = text[:max_len] + f"\n\n[... truncado a {max_len} chars]"
                            results.append({**serp, "content": text, "source": "serp_fallback"})

            # --- Phase 4: Fetch direct URLs ---
            if direct_urls:
                direct_results = await _fetch_urls_parallel(
                    client, direct_urls, headers_extra,
                    fingerprint=fingerprint,
                    mode=mode, max_len=max_len,
                    max_retries=max_retries, request_delay=request_delay,
                    cache_ttl=cache_ttl, context=context,
                    domain_delays=domain_delays,
                )
                results.extend(direct_results)

        successful = sum(1 for r in results if r["success"])

        if successful == 0 and len(results) > 0:
            errors = "; ".join(f"{r['url'][:50]}: {r['content'][:80]}" for r in results[:3])
            raise ValueError(f"All {len(results)} URLs failed. Errors: {errors}")

        # --- Phase 5: Structured extraction via LLM (REGLA-355) ---
        if extraction_schema and successful > 0:
            if not context or not hasattr(context, "llm") or not context.llm:
                raise RuntimeError(
                    "Extraccion estructurada requiere un recurso LLM configurado"
                )
            for r in results:
                if r["success"]:
                    try:
                        extracted = await _extract_structured(
                            context.llm, r["content"], extraction_schema
                        )
                        r["extracted"] = extracted
                    except Exception as exc:
                        r["extraction_error"] = str(exc)

        # --- Phase 6: Auto-vault (REGLA-358) ---
        vault_paths: list[str] = []
        if auto_vault and successful > 0:
            vault = getattr(context, "vault", None) if context else None
            if vault:
                vault_folder = config.get("vault_folder", "vault/scrapes").rstrip("/")
                vault_tags = [t.strip() for t in config.get("vault_tags", "").split(",") if t.strip()]
                for r in results:
                    if r["success"]:
                        try:
                            path = await _save_to_vault(vault, r, vault_folder, vault_tags, context)
                            vault_paths.append(path)
                        except Exception as exc:
                            logger.warning("Auto-vault failed for %s: %s", r["url"][:50], exc)
            else:
                logger.warning("auto_vault=True but VaultService not available")

        return {
            "results": results,
            "total": len(results),
            "successful": successful,
            "vault_paths": vault_paths,
        }


# ---------------------------------------------------------------------------
# Parallel fetcher — stealth-first (FEAT-023)
# ---------------------------------------------------------------------------

# Max domain backoff cap — REGLA-384
_MAX_DOMAIN_DELAY = 30.0

async def _fetch_urls_parallel(
    client: Any,
    urls: list[str],
    headers_extra: dict,
    *,
    fingerprint: SessionFingerprint | None = None,
    mode: str = "text",
    max_len: int = 0,
    concurrency: int = 5,
    max_retries: int = 3,
    request_delay: float = 0.5,
    cache_ttl: int = 0,
    context: Any = None,
    domain_delays: dict[str, float] | None = None,
) -> list[dict[str, Any]]:
    """Fetch multiple URLs concurrently with stealth, retry, cache, and rate limiting."""
    semaphore = asyncio.Semaphore(concurrency)
    domain_locks: dict[str, asyncio.Lock] = {}
    if domain_delays is None:
        domain_delays = {}
    # Referer tracking — REGLA-386
    last_url_per_domain: dict[str, str] = {}

    async def _get_domain_lock(url: str) -> asyncio.Lock:
        domain = urlparse(url).netloc.lower()
        if domain not in domain_locks:
            domain_locks[domain] = asyncio.Lock()
        return domain_locks[domain]

    async def _fetch_one(i: int, url: str) -> dict[str, Any]:
        async with semaphore:
            # Check cache (REGLA-356)
            if cache_ttl > 0 and context and hasattr(context, "db"):
                cached = await _check_cache(context.db, url, cache_ttl)
                if cached:
                    return cached

            domain = urlparse(url).netloc.lower()

            # Build referer — REGLA-386
            referer = last_url_per_domain.get(domain)

            # Per-domain rate limiting (REGLA-359) + domain backoff (REGLA-384)
            lock = await _get_domain_lock(url)
            async with lock:
                # Apply domain-specific delay if rate-limited before
                extra_delay = domain_delays.get(domain, 0.0)
                if extra_delay > 0:
                    await asyncio.sleep(apply_jitter(extra_delay))

                result = await _fetch_with_retry(
                    client, url, headers_extra,
                    fingerprint=fingerprint,
                    referer=referer,
                    mode=mode, max_len=max_len, max_retries=max_retries,
                    domain_delays=domain_delays,
                )

                # Track referer chain — REGLA-386
                if result["success"]:
                    last_url_per_domain[domain] = url

                if request_delay > 0:
                    await asyncio.sleep(apply_jitter(request_delay))

            # Update cache
            if cache_ttl > 0 and context and hasattr(context, "db") and result["success"]:
                try:
                    await _update_cache(context.db, url, result, cache_ttl)
                except Exception:
                    pass

            return result

    tasks = [_fetch_one(i, url) for i, url in enumerate(urls)]
    return list(await asyncio.gather(*tasks))


async def _fetch_with_retry(
    client: Any,
    url: str,
    headers_extra: dict,
    *,
    fingerprint: SessionFingerprint | None = None,
    referer: str | None = None,
    mode: str = "text",
    max_len: int = 0,
    max_retries: int = 3,
    domain_delays: dict[str, float] | None = None,
) -> dict[str, Any]:
    """Fetch a URL with stealth headers and exponential backoff. REGLA-357/383."""
    base_delay = 1.0
    domain = urlparse(url).netloc.lower()

    # Build headers from fingerprint (REGLA-383: always stealth)
    if fingerprint:
        request_headers = dict(fingerprint.headers)
    else:
        request_headers = dict(SessionFingerprint.generate().headers)

    # Referer chain — REGLA-386
    if referer:
        request_headers["Referer"] = referer
        request_headers["Sec-Fetch-Site"] = "same-origin" if urlparse(referer).netloc == urlparse(url).netloc else "cross-site"

    # Merge user-provided headers (override stealth if explicit)
    request_headers.update(headers_extra)

    for attempt in range(max_retries + 1):
        try:
            resp = await client.get(url, headers=request_headers)

            # Rate limited — REGLA-384: cumulative domain backoff
            if resp.status_code == 429:
                if domain_delays is not None:
                    current = domain_delays.get(domain, base_delay)
                    domain_delays[domain] = min(current * 2, _MAX_DOMAIN_DELAY)
                if attempt < max_retries:
                    retry_after = resp.headers.get("Retry-After")
                    if retry_after and retry_after.isdigit():
                        delay = int(retry_after)
                    else:
                        delay = base_delay * (2 ** attempt)
                    await asyncio.sleep(apply_jitter(delay))
                    continue

            # 403 also triggers domain backoff
            if resp.status_code == 403 and domain_delays is not None:
                current = domain_delays.get(domain, base_delay)
                domain_delays[domain] = min(current * 1.5, _MAX_DOMAIN_DELAY)

            raw_html = resp.text
            title = _extract_title(raw_html)

            if mode == "html":
                content = raw_html
            elif mode == "markdown":
                try:
                    from datamirai_engine.tools.builtin.data.html_to_markdown import html_to_markdown
                    content = html_to_markdown(raw_html)
                except Exception:
                    content = _html_to_text(raw_html)
            else:
                content = _html_to_text(raw_html)

            if max_len and len(content) > max_len:
                content = content[:max_len] + f"\n\n[... truncado a {max_len} chars]"

            return {
                "url": url,
                "title": title,
                "content": content,
                "status_code": resp.status_code,
                "success": 200 <= resp.status_code < 400,
            }
        except Exception as exc:
            if attempt < max_retries:
                delay = base_delay * (2 ** attempt) + random.random()
                await asyncio.sleep(delay)
                continue
            return {
                "url": url,
                "title": "",
                "content": str(exc),
                "status_code": 0,
                "success": False,
            }

    return {"url": url, "title": "", "content": "Max retries exceeded", "status_code": 0, "success": False}


# ---------------------------------------------------------------------------
# Search engine helpers
# ---------------------------------------------------------------------------

def _build_search_urls(query: str, engines: str, date_range: str = "any") -> list[str]:
    """Generate search engine URLs from a query string with optional date filtering."""
    q = quote_plus(query)
    engine_list = [e.strip().lower() for e in engines.split(",") if e.strip()]

    # Date filter params per engine
    _date_params: dict[str, dict[str, str]] = {
        "google":     {"day": "&tbs=qdr:d", "week": "&tbs=qdr:w", "month": "&tbs=qdr:m"},
        "bing":       {"day": "&freshness=Day", "week": "&freshness=Week", "month": "&freshness=Month"},
        "duckduckgo": {"day": "&df=d", "week": "&df=w", "month": "&df=m"},
        "brave":      {"day": "&tf=pd", "week": "&tf=pw", "month": "&tf=pm"},
    }

    base_urls = {
        "google":     f"https://www.google.com/search?q={q}&hl=es",
        "bing":       f"https://www.bing.com/search?q={q}",
        "duckduckgo": f"https://html.duckduckgo.com/html/?q={q}",
        "brave":      f"https://search.brave.com/search?q={q}",
    }

    urls: list[str] = []
    for engine in engine_list:
        if engine not in base_urls:
            continue
        url = base_urls[engine]
        if date_range != "any" and engine in _date_params and date_range in _date_params[engine]:
            url += _date_params[engine][date_range]
        urls.append(url)
    return urls


def _detect_engine(url: str) -> str:
    """Detect which search engine a URL belongs to."""
    domain = urlparse(url).netloc.lower()
    if "google" in domain:
        return "google"
    if "bing" in domain:
        return "bing"
    if "duckduckgo" in domain:
        return "duckduckgo"
    if "brave" in domain:
        return "brave"
    return "unknown"


def _extract_serp_links(html: str, engine: str, max_links: int = 8) -> list[str]:
    """Extract real article links from a search engine result page."""
    links: list[str] = []

    if engine == "google":
        # Google wraps result links in /url?q=<actual_url>&...
        for match in re.finditer(r'/url\?q=(https?://[^&"\']+)', html):
            url = unquote(match.group(1))
            if _is_article_url(url) and url not in links:
                links.append(url)

    elif engine == "duckduckgo":
        # DDG HTML version uses uddg= redirect param
        for match in re.finditer(r'uddg=(https?[^&"\']+)', html):
            url = unquote(match.group(1))
            if _is_article_url(url) and url not in links:
                links.append(url)
        # Fallback: direct hrefs on result anchors
        if not links:
            for match in re.finditer(r'class="result__a"[^>]*href="(https?://[^"]+)"', html):
                url = unquote(match.group(1))
                if _is_article_url(url) and url not in links:
                    links.append(url)

    elif engine == "bing":
        # Bing: article links inside <li class="b_algo">...<a href="...">
        for match in re.finditer(r'class="b_algo"[^>]*>.*?<a\s+href="(https?://[^"]+)"', html, re.DOTALL):
            url = unquote(match.group(1))
            if _is_article_url(url) and url not in links:
                links.append(url)
        # Fallback: any external https link
        if not links:
            for match in re.finditer(r'href="(https?://[^"]+)"', html):
                url = unquote(match.group(1))
                if _is_article_url(url) and url not in links:
                    links.append(url)

    else:
        # Generic: extract all external links
        for match in re.finditer(r'href="(https?://[^"]+)"', html):
            url = unquote(match.group(1))
            if _is_article_url(url) and url not in links:
                links.append(url)

    return links[:max_links]


def _is_article_url(url: str) -> bool:
    """Check if a URL looks like a real article (not a search engine or utility page)."""
    try:
        parsed = urlparse(url)
        domain = parsed.netloc.lower()
    except Exception:
        return False

    for skip in _SKIP_DOMAINS:
        if skip in domain:
            return False

    path = parsed.path.lower()
    _skip_paths = ("/search", "/images", "/maps", "/login", "/signup", "/privacy", "/terms", "/cookie")
    if any(path.startswith(p) for p in _skip_paths):
        return False

    _skip_exts = (".pdf", ".zip", ".exe", ".dmg", ".jpg", ".png", ".gif", ".svg", ".css", ".js")
    if any(path.endswith(ext) for ext in _skip_exts):
        return False

    return True


# ---------------------------------------------------------------------------
# HTML helpers
# ---------------------------------------------------------------------------

def _extract_title(html: str) -> str:
    """Extract <title> content from HTML safely."""
    lower = html.lower()
    start_idx = lower.find("<title>")
    if start_idx == -1:
        return ""
    start_idx += 7
    end_idx = lower.find("</title>", start_idx)
    if end_idx == -1:
        return html[start_idx:start_idx + 200].strip()
    return html[start_idx:end_idx].strip()


# ---------------------------------------------------------------------------
# v2: Cache helpers (REGLA-356)
# ---------------------------------------------------------------------------

# Tracking params to strip for cache key normalization
_TRACKING_PARAMS = frozenset({
    "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content",
    "fbclid", "gclid", "ref", "source", "mc_cid", "mc_eid",
})


def _normalize_url(url: str) -> str:
    """Normalize URL for cache key: lowercase host, sort params, strip tracking."""
    parsed = urlparse(url)
    params = parse_qs(parsed.query, keep_blank_values=True)
    # Remove tracking params
    filtered = {k: v for k, v in params.items() if k.lower() not in _TRACKING_PARAMS}
    # Sort remaining
    sorted_query = urlencode(sorted(filtered.items()), doseq=True)
    return f"{parsed.scheme}://{parsed.netloc.lower()}{parsed.path}?{sorted_query}" if sorted_query else f"{parsed.scheme}://{parsed.netloc.lower()}{parsed.path}"


def _url_hash(url: str) -> str:
    """SHA-256 hash of normalized URL."""
    return hashlib.sha256(_normalize_url(url).encode()).hexdigest()[:32]


async def _check_cache(db: Any, url: str, ttl: int) -> dict | None:
    """Check scrape cache for a valid entry."""
    try:
        h = _url_hash(url)
        row = await db.fetch_one(
            "SELECT url, content, status_code, fetched_at FROM scrape_cache WHERE url_hash = ?",
            (h,),
        )
        if not row:
            return None
        # Check TTL
        fetched = datetime.fromisoformat(row["fetched_at"])
        age = (datetime.now(timezone.utc) - fetched).total_seconds()
        if age > ttl:
            return None
        return {
            "url": row["url"],
            "title": "",
            "content": row["content"],
            "status_code": row["status_code"],
            "success": True,
            "cached": True,
        }
    except Exception:
        return None


async def _update_cache(db: Any, url: str, result: dict, ttl: int) -> None:
    """Update scrape cache with a new result."""
    try:
        h = _url_hash(url)
        now = datetime.now(timezone.utc).isoformat()
        await db.execute(
            "CREATE TABLE IF NOT EXISTS scrape_cache "
            "(url_hash TEXT PRIMARY KEY, url TEXT, content TEXT, "
            "status_code INTEGER, fetched_at TEXT, ttl_seconds INTEGER)"
        )
        await db.execute(
            "INSERT OR REPLACE INTO scrape_cache (url_hash, url, content, status_code, fetched_at, ttl_seconds) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (h, url, result["content"], result["status_code"], now, ttl),
        )
    except Exception:
        pass


# ---------------------------------------------------------------------------
# v2: Structured extraction (REGLA-355)
# ---------------------------------------------------------------------------

async def _extract_structured(llm: Any, content: str, schema_json: str) -> dict:
    """Use LLM to extract structured data from content based on a JSON schema."""
    prompt = (
        f"Extract the following fields from the content below. "
        f"Return ONLY valid JSON matching this schema: {schema_json}\n\n"
        f"Content:\n{content[:8000]}\n\n"
        f"JSON output:"
    )
    response = await llm.call(model="", prompt=prompt)
    result_text = response.get("result", "") if isinstance(response, dict) else str(response)
    # Try to parse JSON from the response
    json_match = re.search(r"\{[^{}]*\}", result_text, re.DOTALL)
    if json_match:
        return json.loads(json_match.group(0))
    return {"raw": result_text}


# ---------------------------------------------------------------------------
# v2: Auto-vault (REGLA-358)
# ---------------------------------------------------------------------------

async def _save_to_vault(vault: Any, result: dict, folder: str, tags: list, context: Any) -> str:
    """Save a scrape result as a note in the vault."""
    from datamirai_engine.vault.parser import slugify

    url = result["url"]
    title = result.get("title", "") or urlparse(url).netloc
    slug = slugify(title, max_len=60)
    date = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    path = f"{folder}/{date}-{slug}.md"

    frontmatter = {
        "title": title,
        "source": url,
        "scraped_at": datetime.now(timezone.utc).isoformat(),
        "tags": tags,
    }
    if context:
        if hasattr(context, "session_id") and context.session_id:
            frontmatter["session_id"] = context.session_id
        if hasattr(context, "node_id") and context.node_id:
            frontmatter["node_id"] = context.node_id

    content = result["content"]
    note = await vault.write_note(path, title, content, frontmatter, overwrite=True)
    return note.path


# ---------------------------------------------------------------------------
# HTML helpers
# ---------------------------------------------------------------------------

def _html_to_text(html: str) -> str:
    """HTML to plain-text extractor. Skips nav/footer/script noise."""
    from html.parser import HTMLParser

    _SKIP_TAGS = frozenset({"script", "style", "noscript", "nav", "footer"})
    _BREAK_TAGS = frozenset({"p", "div", "br", "h1", "h2", "h3", "h4", "h5", "li", "tr", "article", "section"})

    class _Stripper(HTMLParser):
        def __init__(self) -> None:
            super().__init__()
            self._parts: list[str] = []
            self._skip_depth = 0

        def handle_starttag(self, tag: str, attrs: list) -> None:
            if tag in _SKIP_TAGS:
                self._skip_depth += 1

        def handle_endtag(self, tag: str) -> None:
            if tag in _SKIP_TAGS:
                self._skip_depth = max(0, self._skip_depth - 1)
            if tag in _BREAK_TAGS:
                self._parts.append("\n")

        def handle_data(self, data: str) -> None:
            if self._skip_depth == 0:
                self._parts.append(data)

    s = _Stripper()
    s.feed(html)
    text = "".join(s._parts)
    lines = [line.strip() for line in text.splitlines()]
    return "\n".join(line for line in lines if line)
