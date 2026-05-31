//! WebScrapeTool -- production-grade stealth scraper

use super::*;
use std::sync::LazyLock;

/// Compiled once: extracts the page `<title>` for scrape results.
static TITLE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)<title[^>]*>(.*?)</title>").unwrap());

// ===========================================================================
// WebScrapeTool -- production-grade stealth scraper
// ===========================================================================

// ---------------------------------------------------------------------------
// Stealth fingerprinting
// ---------------------------------------------------------------------------

/// 12 modern user agents from real browsers (2024-2026).
const USER_AGENT_POOL: &[&str] = &[
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
];

/// 5 common desktop viewport sizes.
const VIEWPORT_POOL: &[(u32, u32)] = &[
    (1920, 1080),
    (1366, 768),
    (1280, 800),
    (1440, 900),
    (1536, 864),
];

/// Deterministic session fingerprint derived from a session_id hash.
#[derive(Debug, Clone)]
pub struct SessionFingerprint {
    pub user_agent: String,
    pub viewport: (u32, u32),
    pub platform: String,
    pub browser: String,
    pub locale: String,
}

impl SessionFingerprint {
    /// Generate a deterministic fingerprint from the given session_id.
    /// The same session_id always produces the same fingerprint.
    pub fn generate(session_id: &str) -> Self {
        use sha2::{Digest, Sha256};

        let hash = Sha256::digest(session_id.as_bytes());
        let seed = u64::from_le_bytes(hash[..8].try_into().unwrap());

        let ua_idx = (seed as usize) % USER_AGENT_POOL.len();
        let vp_idx = ((seed >> 16) as usize) % VIEWPORT_POOL.len();

        let ua = USER_AGENT_POOL[ua_idx];
        let viewport = VIEWPORT_POOL[vp_idx];

        let browser = detect_browser(ua);
        let platform = detect_platform(ua);

        let locales = ["es-ES", "en-US", "es-MX", "en-GB"];
        let locale = locales[((seed >> 32) as usize) % locales.len()];

        Self {
            user_agent: ua.to_string(),
            viewport,
            platform: platform.to_string(),
            browser: browser.to_string(),
            locale: locale.to_string(),
        }
    }

    /// Build HTTP headers from this fingerprint.
    pub fn build_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

        let mut headers = HeaderMap::new();

        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str(&self.user_agent)
                .unwrap_or_else(|_| HeaderValue::from_static("Mozilla/5.0")),
        );

        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            ),
        );

        let accept_lang = format!(
            "{},{};q=0.9,en-US;q=0.8,en;q=0.7",
            self.locale,
            self.locale.split('-').next().unwrap_or("en"),
        );
        if let Ok(val) = HeaderValue::from_str(&accept_lang) {
            headers.insert(reqwest::header::ACCEPT_LANGUAGE, val);
        }

        headers.insert(
            reqwest::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br"),
        );

        headers.insert(
            HeaderName::from_static("dnt"),
            HeaderValue::from_static("1"),
        );

        headers.insert(
            HeaderName::from_static("upgrade-insecure-requests"),
            HeaderValue::from_static("1"),
        );

        // Sec-CH-UA headers only for Chromium-based browsers
        if matches!(self.browser.as_str(), "chrome" | "edge" | "opera") {
            let version = extract_chrome_version(&self.user_agent);

            let sec_ch_ua = match self.browser.as_str() {
                "chrome" => format!(
                    "\"Chromium\";v=\"{version}\", \"Google Chrome\";v=\"{version}\", \"Not.A/Brand\";v=\"24\""
                ),
                "edge" => format!(
                    "\"Chromium\";v=\"{version}\", \"Microsoft Edge\";v=\"{version}\", \"Not.A/Brand\";v=\"24\""
                ),
                "opera" => format!(
                    "\"Chromium\";v=\"{version}\", \"Opera\";v=\"111\", \"Not.A/Brand\";v=\"24\""
                ),
                _ => String::new(),
            };

            if let Ok(val) = HeaderValue::from_str(&sec_ch_ua) {
                headers.insert(HeaderName::from_static("sec-ch-ua"), val);
            }
            headers.insert(
                HeaderName::from_static("sec-ch-ua-mobile"),
                HeaderValue::from_static("?0"),
            );
            let platform_quoted = format!("\"{}\"", self.platform);
            if let Ok(val) = HeaderValue::from_str(&platform_quoted) {
                headers.insert(HeaderName::from_static("sec-ch-ua-platform"), val);
            }
        }

        headers
    }
}

fn detect_browser(ua: &str) -> &'static str {
    if ua.contains("Edg/") {
        "edge"
    } else if ua.contains("OPR/") {
        "opera"
    } else if ua.contains("Firefox/") {
        "firefox"
    } else if ua.contains("Safari/") && !ua.contains("Chrome/") {
        "safari"
    } else if ua.contains("Chrome/") {
        "chrome"
    } else {
        "unknown"
    }
}

fn detect_platform(ua: &str) -> &'static str {
    if ua.contains("Macintosh") {
        "macOS"
    } else if ua.contains("Windows NT") {
        "Windows"
    } else if ua.contains("X11; Linux") || ua.contains("Linux") {
        "Linux"
    } else {
        "Windows"
    }
}

fn extract_chrome_version(ua: &str) -> String {
    let re = regex::Regex::new(r"Chrome/(\d+)").unwrap();
    re.captures(ua)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "125".into())
}

// ---------------------------------------------------------------------------
// Jitter
// ---------------------------------------------------------------------------

/// Apply +/-30% random jitter to a delay value.
pub(crate) fn apply_jitter(delay: Duration) -> Duration {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let factor: f64 = 0.7 + rng.gen::<f64>() * 0.6; // [0.7, 1.3]
    delay.mul_f64(factor)
}

// ---------------------------------------------------------------------------
// URL normalization / cache
// ---------------------------------------------------------------------------

/// Tracking params to strip when normalizing URLs for cache keys.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "ref",
    "mc_cid",
    "mc_eid",
];

/// Normalize a URL: lowercase host, sort query params, strip tracking params.
pub(crate) fn normalize_url(raw: &str) -> String {
    let parsed = match url::Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.to_string(),
    };

    let filtered: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !TRACKING_PARAMS.contains(&k.to_lowercase().as_str()))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let mut sorted = filtered;
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let base = format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or("").to_lowercase(),
        parsed.path()
    );

    if sorted.is_empty() {
        base
    } else {
        let qs: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{base}?{}", qs.join("&"))
    }
}

/// Cached HTTP response.
#[derive(Debug, Clone)]
pub(crate) struct CachedResponse {
    pub(crate) body: String,
    pub(crate) status: u16,
    pub(crate) fetched_at: Instant,
    pub(crate) ttl: Duration,
}

impl CachedResponse {
    pub(crate) fn is_valid(&self) -> bool {
        self.fetched_at.elapsed() < self.ttl
    }
}

// ---------------------------------------------------------------------------
// SERP link extraction
// ---------------------------------------------------------------------------

/// Domains to skip when extracting links from search result pages.
const SKIP_DOMAINS: &[&str] = &[
    "google.com",
    "google.co",
    "gstatic.com",
    "googleapis.com",
    "bing.com",
    "microsoft.com",
    "msn.com",
    "live.com",
    "duckduckgo.com",
    "brave.com",
    "schema.org",
    "w3.org",
    "youtube.com",
    "maps.google.com",
    "facebook.com",
    "instagram.com",
    "apple.com",
    "play.google.com",
];

/// Detect which search engine a URL belongs to.
fn detect_search_engine(url_str: &str) -> Option<&'static str> {
    let lower = url_str.to_lowercase();
    if lower.contains("google.com/search") {
        Some("google")
    } else if lower.contains("duckduckgo.com") {
        Some("duckduckgo")
    } else if lower.contains("bing.com/search") {
        Some("bing")
    } else {
        None
    }
}

/// Check if a URL looks like a real article (not a search engine or utility page).
fn is_article_url(url_str: &str) -> bool {
    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };

    let host = parsed.host_str().unwrap_or("").to_lowercase();

    for skip in SKIP_DOMAINS {
        if host.contains(skip) {
            return false;
        }
    }

    let path = parsed.path().to_lowercase();
    let skip_paths = [
        "/search", "/images", "/maps", "/login", "/signup", "/privacy", "/terms", "/cookie",
    ];
    if skip_paths.iter().any(|p| path.starts_with(p)) {
        return false;
    }

    let skip_exts = [
        ".pdf", ".zip", ".exe", ".dmg", ".jpg", ".png", ".gif", ".svg", ".css", ".js",
    ];
    if skip_exts.iter().any(|ext| path.ends_with(ext)) {
        return false;
    }

    true
}

/// Extract real article links from a SERP HTML page.
pub(crate) fn extract_serp_links(html: &str, engine: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    let max_links = 8;

    match engine {
        "google" => {
            // Google wraps result links in /url?q=<actual_url>&...
            let re = regex::Regex::new(r#"/url\?q=(https?://[^&"']+)"#).unwrap();
            for cap in re.captures_iter(html) {
                if links.len() >= max_links {
                    break;
                }
                let url = urlencoding_decode(&cap[1]);
                if is_article_url(&url) && !links.contains(&url) {
                    links.push(url);
                }
            }
        }
        "duckduckgo" => {
            // DDG uses uddg= redirect param
            let re = regex::Regex::new(r#"uddg=(https?[^&"']+)"#).unwrap();
            for cap in re.captures_iter(html) {
                if links.len() >= max_links {
                    break;
                }
                let url = urlencoding_decode(&cap[1]);
                if is_article_url(&url) && !links.contains(&url) {
                    links.push(url);
                }
            }
            // Fallback: direct hrefs
            if links.is_empty() {
                let re2 =
                    regex::Regex::new(r#"class="result__a"[^>]*href="(https?://[^"]+)""#).unwrap();
                for cap in re2.captures_iter(html) {
                    if links.len() >= max_links {
                        break;
                    }
                    let url = urlencoding_decode(&cap[1]);
                    if is_article_url(&url) && !links.contains(&url) {
                        links.push(url);
                    }
                }
            }
        }
        "bing" => {
            // Bing: article links inside <li class="b_algo">...<a href="...">
            let re = regex::Regex::new(r#"class="b_algo"[^>]*>.*?<a\s+href="(https?://[^"]+)""#)
                .unwrap();
            for cap in re.captures_iter(html) {
                if links.len() >= max_links {
                    break;
                }
                let url = urlencoding_decode(&cap[1]);
                if is_article_url(&url) && !links.contains(&url) {
                    links.push(url);
                }
            }
            // Fallback
            if links.is_empty() {
                let re2 = regex::Regex::new(r#"href="(https?://[^"]+)""#).unwrap();
                for cap in re2.captures_iter(html) {
                    if links.len() >= max_links {
                        break;
                    }
                    let url = urlencoding_decode(&cap[1]);
                    if is_article_url(&url) && !links.contains(&url) {
                        links.push(url);
                    }
                }
            }
        }
        _ => {}
    }

    links
}

/// Minimal URL percent-decoding (handles %XX sequences).
fn urlencoding_decode(s: &str) -> String {
    url::form_urlencoded::parse(s.as_bytes())
        .map(|(k, v)| {
            if v.is_empty() {
                k.to_string()
            } else {
                format!("{k}={v}")
            }
        })
        .collect::<Vec<_>>()
        .join("&")
        // For simple URLs passed as values, just percent-decode directly
        .replace("%3A", ":")
        .replace("%2F", "/")
}

// ---------------------------------------------------------------------------
// Shared state for rate limiting and caching (lazy-static via Arc)
// ---------------------------------------------------------------------------

/// Global scrape state shared across all WebScrapeTool executions.
pub(crate) struct ScrapeState {
    /// Per-domain last-request timestamps for rate limiting.
    domain_delays: Mutex<HashMap<String, Instant>>,
    /// Per-domain backoff multipliers (grows on 429/403).
    domain_backoff: Mutex<HashMap<String, f64>>,
    /// URL-based cache with TTL.
    pub(crate) cache: Mutex<HashMap<String, CachedResponse>>,
}

impl ScrapeState {
    pub(crate) fn new() -> Self {
        Self {
            domain_delays: Mutex::new(HashMap::new()),
            domain_backoff: Mutex::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
        }
    }
}

/// Lazy global state for the scraper. Shared across all invocations.
static SCRAPE_STATE: std::sync::OnceLock<Arc<ScrapeState>> = std::sync::OnceLock::new();

fn get_scrape_state() -> Arc<ScrapeState> {
    SCRAPE_STATE
        .get_or_init(|| Arc::new(ScrapeState::new()))
        .clone()
}

// ---------------------------------------------------------------------------
// Search engine URL builders
// ---------------------------------------------------------------------------

/// Build a search engine URL for the given query.
fn build_search_url(engine: &str, query: &str, date_range: &str) -> Option<String> {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", query)
        .finish();

    match engine {
        "google" => {
            let mut url = format!("https://www.google.com/search?{encoded}");
            match date_range {
                "day" => url.push_str("&tbs=qdr:d"),
                "week" => url.push_str("&tbs=qdr:w"),
                "month" => url.push_str("&tbs=qdr:m"),
                "year" => url.push_str("&tbs=qdr:y"),
                _ => {}
            }
            Some(url)
        }
        "bing" => {
            let mut url = format!("https://www.bing.com/search?{encoded}");
            match date_range {
                "day" => url.push_str("&filters=ex1%3a%22ez1%22"),
                "week" => url.push_str("&filters=ex1%3a%22ez2%22"),
                "month" => url.push_str("&filters=ex1%3a%22ez3%22"),
                _ => {}
            }
            Some(url)
        }
        "duckduckgo" => {
            let mut url = format!("https://html.duckduckgo.com/html/?{encoded}");
            match date_range {
                "day" => url.push_str("&df=d"),
                "week" => url.push_str("&df=w"),
                "month" => url.push_str("&df=m"),
                "year" => url.push_str("&df=y"),
                _ => {}
            }
            Some(url)
        }
        _ => None,
    }
}

/// Fetch a single URL using stealth + cache + rate limiting + retry.
/// Returns (body, status, cached).
async fn fetch_single_url(
    url_str: &str,
    fingerprint: &SessionFingerprint,
    state: &ScrapeState,
    max_retries: u32,
    timeout_secs: u64,
    cache_ttl: Duration,
) -> Result<(String, u16, bool), String> {
    // Check cache
    let normalized = normalize_url(url_str);
    {
        let cache = state.cache.lock().await;
        if let Some(cached) = cache.get(&normalized) {
            if cached.is_valid() {
                return Ok((cached.body.clone(), cached.status, true));
            }
        }
    }

    // Rate limiting
    let domain = url::Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
        .unwrap_or_default();
    {
        let mut delays = state.domain_delays.lock().await;
        let backoffs = state.domain_backoff.lock().await;
        let base_delay = Duration::from_secs(1);
        if let Some(last) = delays.get(&domain) {
            let backoff_multiplier = backoffs.get(&domain).copied().unwrap_or(1.0);
            let required_delay = base_delay.mul_f64(backoff_multiplier);
            let elapsed = last.elapsed();
            if elapsed < required_delay {
                tokio::time::sleep(apply_jitter(required_delay - elapsed)).await;
            }
        }
        delays.insert(domain.clone(), Instant::now());
    }

    // Build client
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let headers = fingerprint.build_headers();

    // Fetch with retry
    let mut last_status: u16 = 0;
    let mut last_body = String::new();
    let mut success = false;

    for attempt in 0..=max_retries {
        let result = client.get(url_str).headers(headers.clone()).send().await;
        match result {
            Ok(response) => {
                last_status = response.status().as_u16();
                let should_retry = matches!(last_status, 429 | 500 | 502 | 503 | 504);
                if last_status == 429 || last_status == 403 {
                    let mut backoffs = state.domain_backoff.lock().await;
                    let current = backoffs.get(&domain).copied().unwrap_or(1.0);
                    backoffs.insert(domain.clone(), (current * 2.0).min(30.0));
                }
                if should_retry && attempt < max_retries {
                    let base = Duration::from_secs(1u64 << attempt.min(4));
                    tokio::time::sleep(apply_jitter(base)).await;
                    continue;
                }
                last_body = response.text().await.unwrap_or_default();
                success = (200..400).contains(&(last_status as i32));
                break;
            }
            Err(_) if attempt < max_retries => {
                let base = Duration::from_secs(1u64 << attempt.min(4));
                tokio::time::sleep(apply_jitter(base)).await;
                continue;
            }
            Err(e) => {
                return Err(format!(
                    "HTTP request failed after {max_retries} retries: {e}"
                ));
            }
        }
    }

    // Cache successful response
    if success {
        let mut cache = state.cache.lock().await;
        cache.insert(
            normalized,
            CachedResponse {
                body: last_body.clone(),
                status: last_status,
                fetched_at: Instant::now(),
                ttl: cache_ttl,
            },
        );
    }

    Ok((last_body, last_status, false))
}

// ---------------------------------------------------------------------------
// WebScrapeTool struct and factory
// ---------------------------------------------------------------------------

data_tool! {
    struct WebScrapeTool, factory WebScrapeFactory;
    tool_type = "data/web_scrape",
    name = "Web Scrape",
    description = "Searches the web or fetches URLs. Supports query-based search (Google/Bing/DuckDuckGo) and direct URL scraping.",
    inputs = [
        field("query", FieldType::String, false, "Search query (searches Google/Bing/DuckDuckGo)"),
        field("url", FieldType::String, false, "Direct URL to fetch (alternative to query)"),
    ],
    outputs = [
        field("results", FieldType::Array, true, "Array of {url, title, content, status_code, success, source}"),
        field("content", FieldType::String, false, "Page content (single URL mode)"),
        field("status", FieldType::Number, false, "HTTP status code (single URL mode)"),
        field("url", FieldType::String, false, "URL fetched (single URL mode)"),
        field("cached", FieldType::Boolean, false, "Whether the response came from cache"),
        field("links", FieldType::Array, false, "Extracted links if URL is a SERP"),
    ],
    config_fields = [
        field("search_engines", FieldType::String, false, "Comma-separated engines: google,bing,duckduckgo (default google)"),
        field("max_results_per_query", FieldType::Number, false, "Max results per search engine (default 5)"),
        field("max_content_length", FieldType::Number, false, "Max chars per result content (default 10000)"),
        field("date_range", FieldType::String, false, "Date range filter: day, week, month, year"),
        field("max_retries", FieldType::Number, false, "Max retries on failure (default 3)"),
        field("timeout_seconds", FieldType::Number, false, "Request timeout in seconds (default 30)"),
        field("cache_ttl_seconds", FieldType::Number, false, "Cache TTL in seconds (default 300)"),
        field("output_schema", FieldType::String, false, "JSON schema for LLM-based structured extraction"),
        field("extract_links", FieldType::Boolean, false, "Whether to extract links from SERP pages (default false)"),
    ]
}

#[async_trait]
impl Tool for WebScrapeTool {
    async fn execute(
        &self,
        inputs: HashMap<String, Value>,
        config: &HashMap<String, Value>,
        context: &dyn ExecutionContext,
    ) -> Result<HashMap<String, Value>, ToolError> {
        let query = inputs.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let url_input = inputs.get("url").and_then(|v| v.as_str()).unwrap_or("");

        if query.is_empty() && url_input.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool_type: "data/web_scrape".into(),
                message: "Either 'query' or 'url' input is required".into(),
            });
        }

        // Read config
        let max_retries = config
            .get("max_retries")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as u32;
        let timeout_secs = config
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);
        let cache_ttl_secs = config
            .get("cache_ttl_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);
        let cache_ttl = Duration::from_secs(cache_ttl_secs);
        let max_results = config
            .get("max_results_per_query")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;
        let max_content_len = config
            .get("max_content_length")
            .and_then(|v| v.as_u64())
            .unwrap_or(10000) as usize;
        let date_range = config
            .get("date_range")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let engines_str = config
            .get("search_engines")
            .and_then(|v| v.as_str())
            .unwrap_or("google");
        let extract_links = config
            .get("extract_links")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let output_schema = config
            .get("output_schema")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let state = get_scrape_state();
        let session_id = context.session_id();
        let fingerprint = SessionFingerprint::generate(session_id);

        // ---------------------------------------------------------------
        // MODE 1: Query-based search (search engines -> extract links -> fetch)
        // ---------------------------------------------------------------
        if !query.is_empty() {
            let engines: Vec<&str> = engines_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            let mut all_results: Vec<Value> = Vec::new();

            for engine in &engines {
                let search_url = match build_search_url(engine, query, date_range) {
                    Some(u) => u,
                    None => continue,
                };

                // Fetch the SERP page
                let (serp_body, serp_status, _cached) = match fetch_single_url(
                    &search_url,
                    &fingerprint,
                    &state,
                    max_retries,
                    timeout_secs,
                    cache_ttl,
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => continue, // skip this engine on error
                };

                if !(200..400).contains(&(serp_status as i32)) {
                    continue;
                }

                // Extract real article links from SERP
                let links = extract_serp_links(&serp_body, engine);
                let links_to_fetch: Vec<&str> =
                    links.iter().map(|s| s.as_str()).take(max_results).collect();

                // Fetch each result page
                for link in links_to_fetch {
                    let (body, status, cached) = fetch_single_url(
                        link,
                        &fingerprint,
                        &state,
                        max_retries,
                        timeout_secs,
                        cache_ttl,
                    )
                    .await
                    .unwrap_or_default();

                    let success = (200..400).contains(&(status as i32));
                    let truncated = if body.len() > max_content_len {
                        &body[..max_content_len]
                    } else {
                        &body
                    };

                    // Try to extract a title from <title> tag
                    let title = TITLE_RE
                        .captures(truncated)
                        .map(|c| c[1].trim().to_string())
                        .unwrap_or_default();

                    all_results.push(json!({
                        "url": link,
                        "title": title,
                        "content": truncated,
                        "status_code": status,
                        "success": success,
                        "cached": cached,
                        "source": *engine,
                    }));
                }
            }

            let mut out = HashMap::new();
            out.insert("results".to_string(), json!(all_results));
            return Ok(out);
        }

        // ---------------------------------------------------------------
        // MODE 2: Direct URL fetch
        // ---------------------------------------------------------------
        let (last_body, last_status, cached) = fetch_single_url(
            url_input,
            &fingerprint,
            &state,
            max_retries,
            timeout_secs,
            cache_ttl,
        )
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool_type: "data/web_scrape".into(),
            message: e,
        })?;

        let success = (200..400).contains(&(last_status as i32));

        // SERP link extraction
        let mut extracted_links: Option<Vec<String>> = None;
        if extract_links || detect_search_engine(url_input).is_some() {
            if let Some(engine) = detect_search_engine(url_input) {
                let links = extract_serp_links(&last_body, engine);
                if !links.is_empty() {
                    extracted_links = Some(links);
                }
            }
        }

        // LLM-based structured extraction
        if !output_schema.is_empty() && success {
            let llm = context.llm();
            let body_slice = &last_body[..last_body.len().min(8000)];
            let prompt = format!(
                "Extract the following fields from the content below. \
                 Return ONLY valid JSON matching this schema: {output_schema}\n\n\
                 Content:\n{body_slice}\n\nJSON output:",
            );
            if let Ok(llm_resp) = llm.call("", &prompt, &[], 0.0, 2000).await {
                let re = regex::Regex::new(r"\{[^{}]*\}").unwrap();
                if let Some(m) = re.find(&llm_resp.response) {
                    if let Ok(extracted) = serde_json::from_str::<Value>(m.as_str()) {
                        let mut out = HashMap::new();
                        out.insert("content".to_string(), json!(last_body));
                        out.insert("status".to_string(), json!(last_status));
                        out.insert("url".to_string(), json!(url_input));
                        out.insert("cached".to_string(), json!(cached));
                        out.insert("extracted".to_string(), extracted);
                        let truncated = &last_body[..last_body.len().min(max_content_len)];
                        out.insert(
                            "results".to_string(),
                            json!([{
                                "url": url_input, "title": "", "content": truncated,
                                "status_code": last_status, "success": success, "source": "direct",
                            }]),
                        );
                        if let Some(links) = extracted_links {
                            out.insert("links".to_string(), json!(links));
                        }
                        return Ok(out);
                    }
                }
            }
        }

        // Build response
        let truncated = &last_body[..last_body.len().min(max_content_len)];
        let mut out = HashMap::new();
        out.insert("content".to_string(), json!(truncated));
        out.insert("status".to_string(), json!(last_status));
        out.insert("url".to_string(), json!(url_input));
        out.insert("cached".to_string(), json!(cached));
        out.insert(
            "results".to_string(),
            json!([{
                "url": url_input, "title": "", "content": truncated,
                "status_code": last_status, "success": success, "source": "direct",
            }]),
        );
        if let Some(links) = extracted_links {
            out.insert("links".to_string(), json!(links));
        }

        Ok(out)
    }
}
