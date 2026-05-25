use std::time::Instant;
use std::{collections::HashSet, env, process::Stdio};

use regex::Regex;
use reqwest::Client as ReqwestClient;
use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use url::Url;

use crate::pipeline::QueryType;
use crate::scrapers::http_client::{HttpClient, HttpClientFactory};
use crate::scrapers::proxy::ProxyPool;
use crate::scrapers::{ScrapedResult, SearchResult, SourceName};

const GOOGLE_SEARCH_FETCH_LIMIT: usize = 30;
const GOOGLE_SEARCH_PAGE_SIZE: usize = 10;

pub async fn scrape(
    factory: HttpClientFactory,
    query: &str,
    query_type: QueryType,
    proxy_pool: Option<&ProxyPool>,
) -> ScrapedResult {
    let started_at = Instant::now();
    let mut diagnostics = Vec::new();

    let browser_primary = env::var("GOOGLE_BROWSER_SIDECAR_ENABLED").ok().as_deref() == Some("1");

    // Phase A: if browser sidecar is set as primary, try it first
    if browser_primary {
        let outcome = scrape_via_browser_sidecar(query, query_type, proxy_pool, started_at).await;
        diagnostics.extend(outcome.diagnostics);
        if let Some(result) = outcome.result {
            return result;
        }
    }

    if env::var("GOOGLE_CLI_SIDECAR_ENABLED").ok().as_deref() == Some("1") {
        let outcome = scrape_via_cli_sidecar(query, query_type, proxy_pool, started_at).await;
        diagnostics.extend(outcome.diagnostics);
        if let Some(result) = outcome.result {
            return result;
        }
    }

    // Phase B: HTTP scraping with up to 2 distinct proxies
    // Exclude known-bad sources (same list the browser sidecar service uses)
    let excluded_sources: std::collections::HashSet<String> =
        env::var("GOOGLE_BROWSER_SIDECAR_PROXY_DISABLED_SOURCES")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
    let proxies = proxy_pool
        .map(|pool| pool.pick_distinct_excluding(2, &excluded_sources))
        .unwrap_or_default();

    for proxy in &proxies {
        let client = match factory.google_client(Some(proxy)) {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(
                    proxy_source = %proxy.source_file,
                    error = %error,
                    "failed to build google http client"
                );
                diagnostics.push(json!({
                    "strategy": "http",
                    "proxy_source": proxy.source_file,
                    "success": false,
                    "error": error.to_string(),
                }));
                continue;
            }
        };

        match scrape_once(&client, query, started_at).await {
            Ok(result) if result.success && !result.search_results.is_empty() => {
                tracing::info!(
                    proxy_source = %proxy.source_file,
                    result_count = result.search_results.len(),
                    "google http scrape succeeded"
                );
                return result;
            }
            Ok(result) => {
                let reason = google_failure_reason_from_metadata(
                    &result.metadata,
                    result.search_results.len(),
                );
                tracing::warn!(
                    proxy_source = %proxy.source_file,
                    reason = %reason,
                    "google http scrape failed"
                );
                diagnostics.push(json!({
                    "strategy": "http",
                    "proxy_source": proxy.source_file,
                    "success": false,
                    "error": reason,
                    "metadata": result.metadata,
                    "result_count": result.search_results.len(),
                }));
            }
            Err(error) => {
                tracing::warn!(
                    proxy_source = %proxy.source_file,
                    error = %error,
                    "google http scrape transport error"
                );
                diagnostics.push(json!({
                    "strategy": "http",
                    "proxy_source": proxy.source_file,
                    "success": false,
                    "error": error.to_string(),
                }));
            }
        }
    }

    // Phase C: auto-fallback to browser sidecar (if not already used as primary)
    if !browser_primary {
        tracing::info!("all http attempts failed, falling back to browser sidecar");
        let outcome = scrape_via_browser_sidecar(query, query_type, proxy_pool, started_at).await;
        diagnostics.extend(outcome.diagnostics);
        if let Some(result) = outcome.result {
            return result;
        }
    }

    let mut failure = ScrapedResult::failure(
        SourceName::Google,
        query,
        started_at,
        build_google_failure_message(&diagnostics),
    );
    failure.metadata = json!({
        "attempts": diagnostics,
        "result_count": 0,
    });
    failure
}

async fn scrape_once(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let mut aggregated_results = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut raw_html_samples = Vec::new();
    let mut last_block_kind: Option<&'static str> = None;
    let mut pages_attempted = 0usize;
    let mut pages_succeeded = 0usize;

    for page_start in (0..GOOGLE_SEARCH_FETCH_LIMIT).step_by(GOOGLE_SEARCH_PAGE_SIZE) {
        pages_attempted += 1;
        // Primary: gbv=1 (basic HTML, no JS required, works with desktop UAs)
        let body = fetch_google_basic_results_page(client, query, page_start).await?;
        let block_kind = detect_google_block_kind(&body);
        let blocked = block_kind.is_some();
        let mut parsed_results = if !blocked {
            parse_google_basic_results(&body)
        } else {
            Vec::new()
        };
        if raw_html_samples.len() < 2 {
            raw_html_samples.push(trim_preview(&body, 1_000));
        }

        // Fallback: extract redirect URL from noscript meta refresh
        if parsed_results.is_empty() && !blocked {
            if let Some(basic_results_url) = extract_google_basic_results_url(&body) {
                let redirect_body = client.get_text(&basic_results_url).await?;
                if raw_html_samples.len() < 2 {
                    raw_html_samples.push(trim_preview(&redirect_body, 1_000));
                }
                if !is_google_block_page(&redirect_body) {
                    parsed_results = parse_google_basic_results(&redirect_body);
                }
            }
        }

        if blocked {
            last_block_kind = block_kind;
            break;
        }

        pages_succeeded += 1;
        for item in parsed_results {
            let dedupe_key = canonicalize_result_url(&item.url);
            if seen_urls.insert(dedupe_key) {
                aggregated_results.push(item);
            }
            if aggregated_results.len() >= GOOGLE_SEARCH_FETCH_LIMIT {
                break;
            }
        }
        if aggregated_results.len() >= GOOGLE_SEARCH_FETCH_LIMIT {
            break;
        }
    }

    let google_status = if pages_succeeded == 0 {
        last_block_kind.unwrap_or("unknown")
    } else {
        "serp"
    };
    let content_status = if google_status == "serp" {
        if aggregated_results.is_empty() {
            "empty_results"
        } else {
            "has_results"
        }
    } else {
        "unknown"
    };

    let mut result = ScrapedResult::success(SourceName::Google, query, started_at);
    result.search_results = aggregated_results;
    result.metadata = json!({
        "mode": "http",
        "query_attempted": query,
        "transport_status": "ok",
        "google_status": google_status,
        "content_status": content_status,
        "pages_attempted": pages_attempted,
        "pages_succeeded": pages_succeeded,
        "result_count": result.search_results.len(),
    });
    result.raw_html = Some(raw_html_samples.join("\n\n---\n\n"));
    result.success = !result.search_results.is_empty() && google_status == "serp";
    Ok(result)
}

fn collect_google_tch1_fragments(body: &str) -> Vec<String> {
    serde_json::Deserializer::from_str(body)
        .into_iter::<Value>()
        .filter_map(Result::ok)
        .filter_map(|value| value.get("d").and_then(Value::as_str).map(str::to_owned))
        .collect::<Vec<_>>()
}

fn parse_google_basic_results(body: &str) -> Vec<SearchResult> {
    parse_html_fragments(body)
}

async fn fetch_google_basic_results_page(
    client: &HttpClient,
    search_query: &str,
    page_start: usize,
) -> anyhow::Result<String> {
    client
        .get_text_with_query(
            "https://www.google.com/search",
            &[
                ("q", search_query.to_owned()),
                ("hl", "vi".to_owned()),
                ("gl", "vn".to_owned()),
                ("gbv", "1".to_owned()),
                ("num", GOOGLE_SEARCH_PAGE_SIZE.to_string()),
                ("start", page_start.to_string()),
            ],
        )
        .await
}

fn parse_html_fragments(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_fragment(html);
    let title_selector = Selector::parse("h3").expect("valid selector");
    let snippet_selector =
        Selector::parse(
            "div.VwiC3b, div[data-sncf='1'], span.aCOpRe, div.s3v9rd, div.yXK7lf, div.BNeawe.s3v9rd, div.uhHOwf.BYbUcd"
        )
            .expect("valid selector");

    document
        .select(&title_selector)
        .filter_map(|heading| {
            let title = heading.text().collect::<String>().trim().to_owned();
            let href = find_parent_href(&heading)?;
            let snippet = find_nearest_snippet(&heading, &snippet_selector);
            Some(SearchResult {
                title,
                url: normalize_google_url(&href),
                snippet,
            })
        })
        .filter(|item| item.url.starts_with("http://") || item.url.starts_with("https://"))
        .take(GOOGLE_SEARCH_FETCH_LIMIT)
        .collect()
}

fn find_parent_href(heading: &ElementRef<'_>) -> Option<String> {
    for ancestor in heading.ancestors() {
        let element = ElementRef::wrap(ancestor)?;
        if element.value().name() != "a" {
            continue;
        }
        if let Some(href) = element.value().attr("href") {
            return Some(href.to_owned());
        }
    }
    None
}

fn find_nearest_snippet(heading: &ElementRef<'_>, snippet_selector: &Selector) -> String {
    for ancestor in heading.ancestors() {
        let Some(element) = ElementRef::wrap(ancestor) else {
            continue;
        };
        if let Some(snippet) = element.select(snippet_selector).find_map(|item| {
            let text = item.text().collect::<String>();
            let trimmed = text.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_owned())
        }) {
            return snippet;
        }
    }
    String::new()
}

fn normalize_google_url(href: &str) -> String {
    if let Some(raw) = href.strip_prefix("/url?q=") {
        if let Ok(url) = Url::parse(&format!("https://www.google.com/url?q={raw}")) {
            if let Some(value) = url
                .query_pairs()
                .find(|(key, _)| key == "q")
                .map(|(_, value)| value.to_string())
            {
                return value;
            }
        }
    }
    href.to_owned()
}

fn canonicalize_result_url(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_owned();
    };

    parsed.set_fragment(None);

    let filtered_pairs = parsed
        .query_pairs()
        .filter(|(key, _)| !is_tracking_query_param(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed
        .query_pairs_mut()
        .clear()
        .extend_pairs(filtered_pairs);

    let query_is_empty = parsed.query().is_none_or(str::is_empty);
    if query_is_empty {
        parsed.set_query(None);
    }

    parsed.to_string()
}

fn is_tracking_query_param(key: &str) -> bool {
    key.starts_with("utm_")
        || matches!(
            key,
            "fbclid"
                | "gclid"
                | "dclid"
                | "gbraid"
                | "wbraid"
                | "ved"
                | "ei"
                | "sei"
                | "sa"
                | "usg"
                | "aqs"
                | "oq"
                | "source"
                | "sxsrf"
                | "sca_esv"
                | "rlz"
                | "iflsig"
                | "sclient"
                | "qh_ss"
                | "srsltid"
        )
}

fn is_google_block_page(body: &str) -> bool {
    detect_google_block_kind(body).is_some()
}

fn detect_google_block_kind(body: &str) -> Option<&'static str> {
    let body = body.to_ascii_lowercase();
    for needle in [
        "detected unusual traffic",
        "our systems have detected unusual traffic",
        "g-recaptcha",
        "/sorry/index",
        "id=\"captcha\"",
        "name=\"captcha\"",
        "captcha-form",
    ] {
        if body.contains(needle) {
            return Some("captcha");
        }
    }
    if body.contains("consent.google") {
        return Some("blocked");
    }
    if body.contains("/httpservice/retry/enablejs") {
        return Some("enablejs");
    }
    None
}

fn extract_google_basic_results_url(body: &str) -> Option<String> {
    if let Some(path) = extract_google_basic_results_path_from_raw_body(body) {
        return Url::parse("https://www.google.com")
            .ok()?
            .join(&path)
            .ok()
            .map(|url| url.to_string());
    }

    let fragments = collect_google_tch1_fragments(body);
    let combined = fragments.join("");
    let document = Html::parse_document(&combined);
    let selector =
        Selector::parse("noscript meta[http-equiv=\"refresh\"], meta[http-equiv=\"refresh\"]")
            .ok()?;
    let content = document
        .select(&selector)
        .next()
        .and_then(|node| node.value().attr("content"))?;
    let path = decode_google_escaped_path(content.split_once("url=")?.1.trim());
    Url::parse("https://www.google.com")
        .ok()?
        .join(&path)
        .ok()
        .map(|url| url.to_string())
}

fn extract_google_basic_results_path_from_raw_body(body: &str) -> Option<String> {
    let pattern = Regex::new(r#"(/search\?q=.*?gbv=1.*?sei=[^"'\\<\s]+)"#).ok()?;
    let raw = pattern.captures(body)?.get(1)?.as_str();
    Some(decode_google_escaped_path(raw))
}

fn decode_google_escaped_path(value: &str) -> String {
    value
        .replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("\\u003d", "=")
        .replace("\\u003f", "?")
        .replace("\\u0025", "%")
        .replace("&amp;", "&")
        .trim_end_matches('\\')
        .to_string()
}

fn trim_preview(body: &str, max_chars: usize) -> String {
    body.chars().take(max_chars).collect()
}

#[derive(Debug, Deserialize)]
struct GoogleCliPayload {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    search_results: Vec<SearchResult>,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    raw_html: String,
}

#[derive(Debug, Default)]
struct SidecarAttemptOutcome {
    result: Option<ScrapedResult>,
    diagnostics: Vec<Value>,
}

async fn scrape_via_cli_sidecar(
    query: &str,
    query_type: QueryType,
    proxy_pool: Option<&ProxyPool>,
    started_at: Instant,
) -> SidecarAttemptOutcome {
    let mut outcome = SidecarAttemptOutcome::default();

    let direct = run_cli_sidecar(query, query_type, None).await;
    if let Some(result) = record_sidecar_attempt(
        "cli_sidecar",
        false,
        query,
        started_at,
        direct,
        &mut outcome,
    ) {
        outcome.result = Some(result);
        return outcome;
    }

    let proxy = proxy_pool
        .and_then(ProxyPool::pick)
        .map(|item| item.url.clone());
    if let Some(proxy) = proxy {
        let via_proxy = run_cli_sidecar(query, query_type, Some(proxy)).await;
        if let Some(result) = record_sidecar_attempt(
            "cli_sidecar",
            true,
            query,
            started_at,
            via_proxy,
            &mut outcome,
        ) {
            outcome.result = Some(result);
        }
    }

    outcome
}

async fn scrape_via_browser_sidecar(
    query: &str,
    query_type: QueryType,
    _proxy_pool: Option<&ProxyPool>,
    started_at: Instant,
) -> SidecarAttemptOutcome {
    let mut outcome = SidecarAttemptOutcome::default();

    // Service handles proxy rotation internally via its own proxy registry.
    // A single call without explicit proxy lets the service try all healthy proxies.
    let payload = run_browser_sidecar(query, query_type, None).await;
    if let Some(result) = record_sidecar_attempt(
        "browser_sidecar",
        false,
        query,
        started_at,
        payload,
        &mut outcome,
    ) {
        outcome.result = Some(result);
    }

    outcome
}

fn payload_to_scraped_result(
    query: &str,
    started_at: Instant,
    result: GoogleCliPayload,
) -> ScrapedResult {
    let mut payload = ScrapedResult::success(SourceName::Google, query, started_at);
    payload.search_results = result.search_results;
    payload.metadata = result.metadata;
    payload.raw_html = (!result.raw_html.is_empty()).then_some(result.raw_html);
    payload
}

fn record_sidecar_attempt(
    strategy: &str,
    proxy_used: bool,
    query: &str,
    started_at: Instant,
    payload: anyhow::Result<GoogleCliPayload>,
    outcome: &mut SidecarAttemptOutcome,
) -> Option<ScrapedResult> {
    match payload {
        Ok(payload) => {
            let result_count = payload.search_results.len();
            let success = payload.success && result_count > 0;
            let error = (!success)
                .then(|| google_failure_reason_from_metadata(&payload.metadata, result_count));
            outcome.diagnostics.push(json!({
                "strategy": strategy,
                "proxy_used": proxy_used,
                "success": success,
                "result_count": result_count,
                "metadata": payload.metadata,
                "error": error,
            }));
            if success {
                Some(payload_to_scraped_result(query, started_at, payload))
            } else {
                None
            }
        }
        Err(error) => {
            outcome.diagnostics.push(json!({
                "strategy": strategy,
                "proxy_used": proxy_used,
                "success": false,
                "error": error.to_string(),
            }));
            None
        }
    }
}

fn google_failure_reason_from_metadata(metadata: &Value, result_count: usize) -> String {
    if let Some(error) = metadata.get("error").and_then(Value::as_str) {
        let trimmed = error.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    match metadata.get("google_status").and_then(Value::as_str) {
        Some("enablejs") => "google returned enablejs challenge".to_owned(),
        Some("captcha") => "google returned captcha challenge".to_owned(),
        Some("blocked") => "google blocked the request".to_owned(),
        Some("serp") if result_count == 0 => "google returned empty serp".to_owned(),
        Some(status) => format!("google returned unusable status: {status}"),
        None if result_count == 0 => "google returned no usable results".to_owned(),
        None => "google returned unusable payload".to_owned(),
    }
}

fn build_google_failure_message(diagnostics: &[Value]) -> String {
    diagnostics
        .iter()
        .rev()
        .find_map(|attempt| attempt.get("error").and_then(Value::as_str))
        .filter(|error| !error.trim().is_empty())
        .map(|error| format!("google search failed: {error}"))
        .unwrap_or_else(|| "google search returned no usable results".to_owned())
}

async fn run_cli_sidecar(
    query: &str,
    query_type: QueryType,
    proxy: Option<String>,
) -> anyhow::Result<GoogleCliPayload> {
    let python = env::var("GOOGLE_SCRAPER_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let script = env::var("GOOGLE_SCRAPER_CLI_PATH")
        .unwrap_or_else(|_| "scripts/google_search_cli.py".to_string());

    let mut command = Command::new(python);
    command
        .arg(script)
        .arg("--query")
        .arg(query)
        .arg("--query-type")
        .arg(query_type.as_str())
        .arg("--limit")
        .arg(GOOGLE_SEARCH_FETCH_LIMIT.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    if let Some(proxy) = proxy {
        command.arg("--proxy").arg(proxy);
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
        .await
        .map_err(|_| anyhow::anyhow!("google cli sidecar timed out"))??;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(anyhow::anyhow!(
            "google cli sidecar returned no JSON output"
        ));
    }

    Ok(serde_json::from_slice::<GoogleCliPayload>(&output.stdout)?)
}

async fn run_browser_sidecar(
    query: &str,
    query_type: QueryType,
    proxy: Option<String>,
) -> anyhow::Result<GoogleCliPayload> {
    if let Ok(service_url) = env::var("GOOGLE_BROWSER_SIDECAR_URL") {
        return run_browser_sidecar_via_service(&service_url, query, query_type, proxy).await;
    }

    let node = env::var("GOOGLE_BROWSER_SIDECAR_NODE").unwrap_or_else(|_| "node".to_string());
    let script = env::var("GOOGLE_BROWSER_SIDECAR_PATH")
        .unwrap_or_else(|_| "scripts/google-browser-sidecar/index.mjs".to_string());

    let mut command = Command::new(node);
    command
        .arg(script)
        .arg("--query")
        .arg(query)
        .arg("--query-type")
        .arg(query_type.as_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    if let Some(proxy) = proxy {
        command.arg("--proxy").arg(proxy);
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(45), command.output())
        .await
        .map_err(|_| anyhow::anyhow!("google browser sidecar timed out"))??;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(anyhow::anyhow!(
            "google browser sidecar returned no JSON output"
        ));
    }

    Ok(serde_json::from_slice::<GoogleCliPayload>(&output.stdout)?)
}

async fn run_browser_sidecar_via_service(
    service_url: &str,
    query: &str,
    query_type: QueryType,
    proxy: Option<String>,
) -> anyhow::Result<GoogleCliPayload> {
    let base = service_url.trim_end_matches('/');
    let client = ReqwestClient::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let response = client
        .post(format!("{base}/search"))
        .json(&json!({
            "query": query,
            "queryType": query_type.as_str(),
            "proxy": proxy,
            "limit": GOOGLE_SEARCH_FETCH_LIMIT
        }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| trim_preview(&body, 400));
        return Err(anyhow::anyhow!(
            "google browser sidecar service error: {message}"
        ));
    }

    Ok(serde_json::from_str::<GoogleCliPayload>(&body)?)
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_result_url, collect_google_tch1_fragments, extract_google_basic_results_url,
        fetch_google_basic_results_page, is_google_block_page, normalize_google_url,
        parse_google_basic_results, parse_html_fragments, run_browser_sidecar, run_cli_sidecar,
        scrape, scrape_once,
    };
    use crate::pipeline::QueryType;
    use crate::scrapers::http_client::HttpClientFactory;
    use crate::scrapers::proxy::ProxyPool;
    use std::time::Instant;

    #[test]
    fn parse_google_html_fragments_extracts_mobile_results_without_div_g() {
        let html = r#"
            <div class="MjjYud">
                <div>
                    <a href="/url?q=https://example.com/report&sa=U&ved=2ah">
                        <div><h3>Cảnh báo giao dịch</h3></div>
                    </a>
                    <div class="VwiC3b">Nội dung snippet thử nghiệm về lừa đảo.</div>
                </div>
            </div>
        "#;

        let results = parse_html_fragments(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Cảnh báo giao dịch");
        assert_eq!(results[0].url, "https://example.com/report");
        assert_eq!(
            results[0].snippet,
            "Nội dung snippet thử nghiệm về lừa đảo."
        );
    }

    #[test]
    fn google_results_page_is_not_marked_as_captcha_just_because_support_flag_exists() {
        let body = r#"<script>var hasCaptchaSupport=true;</script><h3>Kết quả bình thường</h3>"#;
        assert!(!is_google_block_page(body));
    }

    #[test]
    fn normalize_google_url_decodes_real_target() {
        let href = "/url?q=https://example.com/path%3Fa%3D1%26b%3D2&sa=U&ved=2ah";
        assert_eq!(
            normalize_google_url(href),
            "https://example.com/path?a=1&b=2"
        );
    }

    #[test]
    fn canonicalize_result_url_drops_tracking_params_for_dedupe() {
        let a = "https://www.adda247.com/jobs/file.pdf?srsltid=aaa&utm_source=foo";
        let b = "https://www.adda247.com/jobs/file.pdf?srsltid=bbb";
        assert_eq!(canonicalize_result_url(a), canonicalize_result_url(b));
    }

    #[test]
    fn extracts_google_basic_results_url_from_tch1_fragments() {
        let body = r#"{"c":1,"d":"<noscript><meta content=\"0;url=/search?q=0562015037\u0026amp;hl=vi\u0026amp;gl=vn\u0026amp;gbv=1\u0026amp;sei=testsei\" http-equiv=\"refresh\"></noscript>"}"#;
        assert_eq!(
            extract_google_basic_results_url(body).as_deref(),
            Some("https://www.google.com/search?q=0562015037&hl=vi&gl=vn&gbv=1&sei=testsei")
        );
    }

    #[test]
    fn parse_google_basic_results_extracts_urls_from_gbv_one_markup() {
        let html = r#"
            <html><body>
                <div>
                    <a href="/url?q=https://checkscam.vn/nguyen-dinh-thang-24/&sa=U">
                        <h3>Nguyen Dinh Thang 0562015037</h3>
                    </a>
                    <div class="BNeawe s3v9rd">Canh bao so 0562015037</div>
                </div>
            </body></html>
        "#;

        let results = parse_google_basic_results(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://checkscam.vn/nguyen-dinh-thang-24/");
        assert_eq!(results[0].snippet, "Canh bao so 0562015037");
    }

    #[tokio::test]
    #[ignore = "network diagnostic for live Google tch=1 scraping"]
    async fn diagnoses_live_google_tch1_strategy_for_phone_query() {
        let query = "0562015037";
        let query_type = QueryType::Phone;
        let factory = HttpClientFactory::default();
        let client = factory.google_client(None).expect("google client");

        let once = scrape_once(&client, query, Instant::now())
            .await
            .expect("scrape_once");
        println!("scrape_once_success={}", once.success);
        println!("scrape_once_metadata={}", once.metadata);
        println!(
            "scrape_once_raw_preview={}",
            once.raw_html
                .as_deref()
                .unwrap_or_default()
                .chars()
                .take(800)
                .collect::<String>()
                .replace('\n', " ")
        );
        let basic_body = fetch_google_basic_results_page(&client, query, 0)
            .await
            .expect("fetch basic body");
        println!(
            "basic_body_preview={}",
            basic_body
                .chars()
                .take(800)
                .collect::<String>()
                .replace('\n', " ")
        );
        let basic_results = parse_google_basic_results(&basic_body);
        println!("basic_results_len={}", basic_results.len());
        for (index, item) in basic_results.iter().enumerate() {
            println!(
                "basic_result[{index}] title={:?} url={} snippet={:?}",
                item.title, item.url, item.snippet
            );
        }
        for (index, item) in once.search_results.iter().enumerate() {
            println!(
                "scrape_once_result[{index}] title={:?} url={} snippet={:?}",
                item.title, item.url, item.snippet
            );
        }

        let full = scrape(factory, query, query_type, None).await;
        println!("scrape_success={}", full.success);
        println!("scrape_error={:?}", full.error);
        println!("scrape_metadata={}", full.metadata);
        for (index, item) in full.search_results.iter().enumerate() {
            println!(
                "scrape_result[{index}] title={:?} url={} snippet={:?}",
                item.title, item.url, item.snippet
            );
        }
    }

    #[tokio::test]
    #[ignore = "network diagnostic for live Google scraping with proxy pool"]
    async fn diagnoses_live_google_strategy_with_proxy_pool() {
        let query = "0562015037";
        let query_type = QueryType::Phone;
        let factory = HttpClientFactory::default();
        let pool = ProxyPool::load_from_dir("proxies").expect("load proxies");
        assert!(!pool.is_empty(), "proxy pool is empty");

        for attempt in 0..6 {
            let proxy = pool.pick().expect("proxy pick").clone();
            let client = factory
                .google_client(Some(&proxy))
                .expect("google client with proxy");
            match scrape_once(&client, query, Instant::now()).await {
                Ok(result) => {
                    println!(
                        "attempt={attempt} proxy_file={} proxy_url={} success={} results={} metadata={} error={:?}",
                        proxy.source_file,
                        proxy.url,
                        result.success,
                        result.search_results.len(),
                        result.metadata,
                        result.error
                    );
                    for (index, item) in result.search_results.iter().take(3).enumerate() {
                        println!(
                            "attempt={attempt} result[{index}] title={:?} url={} snippet={:?}",
                            item.title, item.url, item.snippet
                        );
                    }
                }
                Err(error) => {
                    println!(
                        "attempt={attempt} proxy_file={} proxy_url={} transport_error={error}",
                        proxy.source_file, proxy.url
                    );
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        let pooled = scrape(factory, query, query_type, Some(&pool)).await;
        println!(
            "pooled_success={} pooled_results={} pooled_error={:?} pooled_metadata={}",
            pooled.success,
            pooled.search_results.len(),
            pooled.error,
            pooled.metadata
        );
        for (index, item) in pooled.search_results.iter().take(5).enumerate() {
            println!(
                "pooled_result[{index}] title={:?} url={} snippet={:?}",
                item.title, item.url, item.snippet
            );
        }
    }

    /// Diagnostic: fetches raw tch=1 and gbv=1 responses via a proxy, dumps HTML previews,
    /// and checks whether `collect_google_tch1_fragments` and `parse_html_fragments` find anything.
    ///
    /// Run with: cargo test diagnoses_proxy_html_format -- --nocapture --ignored
    #[tokio::test]
    #[ignore = "network diagnostic: inspect raw HTML returned by proxy to find parser mismatches"]
    async fn diagnoses_proxy_html_format_and_parser_mismatches() {
        const QUERY: &str = "0562015037";
        const PREVIEW_CHARS: usize = 2000;

        // Use a specific ola proxy known to connect without CAPTCHA.
        // Format from proxy_ola.md: host:port:user:pass → http://user:pass@host:port
        let proxy_url = "http://9g8a1:ouhjt@42.96.13.11:32236";
        let factory = HttpClientFactory::default();

        let proxy = crate::scrapers::proxy::ProxyConfig {
            url: proxy_url.to_owned(),
            source_file: "proxy_ola.md".to_owned(),
            rotation_seed: 50, // index ~50 → GSA_USER_AGENTS[50 % 3] = GSA_USER_AGENTS[2]
        };

        let client = factory
            .google_client(Some(&proxy))
            .expect("build google client with proxy");

        // ── 1. Fetch tch=1 response ──────────────────────────────────────────────
        println!("\n=== TCH=1 (Android GSA JSON-fragment endpoint) ===");
        let tch_body = client
            .get_text_with_query(
                "https://www.google.com/search",
                &[
                    ("q", QUERY.to_owned()),
                    ("hl", "vi".to_owned()),
                    ("gl", "vn".to_owned()),
                    ("tch", "1".to_owned()),
                ],
            )
            .await
            .expect("tch=1 fetch");

        let tch_preview: String = tch_body.chars().take(PREVIEW_CHARS).collect();
        println!("tch1_body_len={}", tch_body.len());
        println!("tch1_is_blocked={}", is_google_block_page(&tch_body));
        println!(
            "tch1_starts_with_json={}",
            tch_body.trim_start().starts_with('{') || tch_body.trim_start().starts_with('[')
        );
        println!("tch1_preview:\n{tch_preview}");

        // Check tch=1 JSON fragment parsing
        let fragments = collect_google_tch1_fragments(&tch_body);
        println!("\ntch1_fragment_count={}", fragments.len());
        for (i, frag) in fragments.iter().enumerate().take(5) {
            println!(
                "tch1_fragment[{i}] len={} preview={}",
                frag.len(),
                frag.chars().take(200).collect::<String>()
            );
        }
        let tch_results = parse_html_fragments(&fragments.join(""));
        println!("tch1_parsed_results={}", tch_results.len());
        for (i, item) in tch_results.iter().enumerate() {
            println!("tch1_result[{i}] title={:?} url={}", item.title, item.url);
        }

        // ── 2. Identify what the body actually looks like ────────────────────────
        // Check for h3 tags presence (key signal for parse_html_fragments)
        let h3_count = tch_body.matches("<h3").count();
        let div_g_count = tch_body.matches("class=\"g\"").count();
        let vwic3b_count = tch_body.matches("VwiC3b").count();
        let bneawe_count = tch_body.matches("BNeawe").count();
        println!(
            "\ntch1_h3_tags={h3_count} div_g={div_g_count} VwiC3b={vwic3b_count} BNeawe={bneawe_count}"
        );

        // Check for alternate class names Google may use in tch=1 fragments
        for class in &[
            "MjjYud", "N54PNb", "yuRUbf", "LC20lb", "DKV0Md", "tF2Cxc", "IsZvec", "yDYNvb",
        ] {
            let count = tch_body.matches(class).count();
            if count > 0 {
                println!("tch1_class_count class={class} count={count}");
            }
        }

        // ── 3. Fetch gbv=1 response ──────────────────────────────────────────────
        println!("\n=== GBV=1 (basic HTML fallback) ===");
        let gbv_body = fetch_google_basic_results_page(&client, QUERY, 0)
            .await
            .expect("gbv=1 fetch");

        let gbv_preview: String = gbv_body.chars().take(PREVIEW_CHARS).collect();
        println!("gbv1_body_len={}", gbv_body.len());
        println!("gbv1_is_blocked={}", is_google_block_page(&gbv_body));
        println!("gbv1_preview:\n{gbv_preview}");

        let gbv_results = parse_google_basic_results(&gbv_body);
        println!("gbv1_parsed_results={}", gbv_results.len());
        for (i, item) in gbv_results.iter().enumerate() {
            println!("gbv1_result[{i}] title={:?} url={}", item.title, item.url);
        }

        let gbv_h3_count = gbv_body.matches("<h3").count();
        let gbv_vwic3b = gbv_body.matches("VwiC3b").count();
        let gbv_bneawe = gbv_body.matches("BNeawe").count();
        println!("\ngbv1_h3_tags={gbv_h3_count} VwiC3b={gbv_vwic3b} BNeawe={gbv_bneawe}");
        for class in &[
            "MjjYud", "N54PNb", "yuRUbf", "LC20lb", "DKV0Md", "tF2Cxc", "IsZvec", "yDYNvb",
        ] {
            let count = gbv_body.matches(class).count();
            if count > 0 {
                println!("gbv1_class_count class={class} count={count}");
            }
        }

        // ── 4. No-proxy comparison ───────────────────────────────────────────────
        // Uses GSA UA without proxy → likely CAPTCHA, but useful to compare format
        println!("\n=== NO PROXY (for format comparison) ===");
        let noproxy_client = factory.google_client(None).expect("no-proxy client");
        let noproxy_body = noproxy_client
            .get_text_with_query(
                "https://www.google.com/search",
                &[
                    ("q", QUERY.to_owned()),
                    ("hl", "vi".to_owned()),
                    ("gl", "vn".to_owned()),
                    ("tch", "1".to_owned()),
                ],
            )
            .await
            .expect("no-proxy tch=1");
        println!("noproxy_is_blocked={}", is_google_block_page(&noproxy_body));
        let noproxy_frags = collect_google_tch1_fragments(&noproxy_body);
        println!("noproxy_tch1_fragment_count={}", noproxy_frags.len());
        let noproxy_results = parse_html_fragments(&noproxy_frags.join(""));
        println!("noproxy_tch1_parsed_results={}", noproxy_results.len());

        // ── 5. Summary ───────────────────────────────────────────────────────────
        println!("\n=== SUMMARY ===");
        println!(
            "proxy_tch1_frags={} proxy_tch1_results={}",
            fragments.len(),
            tch_results.len()
        );
        println!("proxy_gbv1_results={}", gbv_results.len());
        println!(
            "noproxy_tch1_frags={} noproxy_tch1_results={}",
            noproxy_frags.len(),
            noproxy_results.len()
        );

        if fragments.is_empty() {
            println!(
                "DIAGNOSIS: tch=1 response is NOT JSON fragment format — body is likely plain HTML or different encoding"
            );
        } else if tch_results.is_empty() {
            println!(
                "DIAGNOSIS: tch=1 fragments parsed but h3/VwiC3b selectors found nothing — class names differ in proxy response"
            );
        }
        if gbv_results.is_empty() && !is_google_block_page(&gbv_body) {
            println!(
                "DIAGNOSIS: gbv=1 HTML not blocked but no results — snippet selector classes (BNeawe s3v9rd etc.) don't match"
            );
        }
    }

    #[tokio::test]
    #[ignore = "network diagnostic for CLI sidecar feasibility"]
    async fn diagnoses_google_cli_sidecar_feasibility() {
        let payload = run_cli_sidecar("0562015037", QueryType::Phone, None)
            .await
            .expect("cli sidecar");
        println!("sidecar_success={}", payload.success);
        println!("sidecar_metadata={}", payload.metadata);
        println!("sidecar_results={}", payload.search_results.len());
        for (index, item) in payload.search_results.iter().take(5).enumerate() {
            println!(
                "sidecar_result[{index}] title={:?} url={} snippet={:?}",
                item.title, item.url, item.snippet
            );
        }
    }

    #[tokio::test]
    #[ignore = "network diagnostic for browser sidecar feasibility"]
    async fn diagnoses_google_browser_sidecar_feasibility() {
        let payload = run_browser_sidecar("0562015037", QueryType::Phone, None)
            .await
            .expect("browser sidecar");
        println!("browser_sidecar_success={}", payload.success);
        println!("browser_sidecar_metadata={}", payload.metadata);
        println!("browser_sidecar_results={}", payload.search_results.len());
        for (index, item) in payload.search_results.iter().take(5).enumerate() {
            println!(
                "browser_sidecar_result[{index}] title={:?} url={} snippet={:?}",
                item.title, item.url, item.snippet
            );
        }
    }
}
