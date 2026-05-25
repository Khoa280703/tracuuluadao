use std::time::Instant;

use reqwest::Client as ReqwestClient;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderMap, HeaderValue, REFERER, USER_AGENT,
};
use scraper::{Html, Selector};
use serde_json::{Value, json};
use url::Url;

use crate::pipeline::QueryType;
use crate::scrapers::http_client::HttpClient;
use crate::scrapers::{ScrapedResult, SearchResult, SourceName};

const DUCKDUCKGO_SEARCH_FETCH_LIMIT: usize = 30;
const DUCKDUCKGO_DESKTOP_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

pub async fn scrape(
    client: Option<HttpClient>,
    query: &str,
    query_type: QueryType,
) -> ScrapedResult {
    let started_at = Instant::now();
    let Some(client) = client else {
        return ScrapedResult::failure(
            SourceName::DuckDuckGo,
            query,
            started_at,
            "http client unavailable",
        );
    };

    match scrape_impl(&client, query, query_type, started_at).await {
        Ok(result) => result,
        Err(error) => {
            ScrapedResult::failure(SourceName::DuckDuckGo, query, started_at, error.to_string())
        }
    }
}

async fn scrape_impl(
    client: &HttpClient,
    query: &str,
    query_type: QueryType,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let search_query = match query_type {
        QueryType::Phone => format!("{query} lừa đảo"),
        QueryType::Bank => format!("\"{query}\" tài khoản ngân hàng lừa đảo"),
        QueryType::Url => format!("\"{query}\" cảnh báo lừa đảo"),
    };
    let mut attempts = Vec::new();
    let mut raw_html_samples = Vec::new();

    match client
        .post_form_text(
            "https://html.duckduckgo.com/html/",
            Default::default(),
            &[("q", &search_query)],
        )
        .await
    {
        Ok(body) => {
            raw_html_samples.push(trim_preview(&body, 2_000));
            let results = parse_results(&body);
            let blocked = detect_duckduckgo_block_kind(&body);
            let title = extract_title(&body);
            attempts.push(json!({
                "strategy": "rquest_post",
                "success": !results.is_empty() && blocked.is_none(),
                "result_count": results.len(),
                "blocked": blocked,
                "title": title,
            }));
            if !results.is_empty() && blocked.is_none() {
                return Ok(build_success_result(
                    query, started_at, results, body, attempts,
                ));
            }
        }
        Err(error) => attempts.push(json!({
            "strategy": "rquest_post",
            "success": false,
            "error": error.to_string(),
        })),
    }

    match fetch_duckduckgo_html_via_reqwest(&search_query).await {
        Ok(body) => {
            if raw_html_samples.len() < 2 {
                raw_html_samples.push(trim_preview(&body, 2_000));
            }
            let results = parse_results(&body);
            let blocked = detect_duckduckgo_block_kind(&body);
            let title = extract_title(&body);
            attempts.push(json!({
                "strategy": "reqwest_post",
                "success": !results.is_empty() && blocked.is_none(),
                "result_count": results.len(),
                "blocked": blocked,
                "title": title,
            }));
            if !results.is_empty() && blocked.is_none() {
                return Ok(build_success_result(
                    query, started_at, results, body, attempts,
                ));
            }
        }
        Err(error) => attempts.push(json!({
            "strategy": "reqwest_post",
            "success": false,
            "error": error.to_string(),
        })),
    }

    let mut failure = ScrapedResult::failure(
        SourceName::DuckDuckGo,
        query,
        started_at,
        build_duckduckgo_failure_message(&attempts),
    );
    failure.raw_html =
        (!raw_html_samples.is_empty()).then_some(raw_html_samples.join("\n\n---\n\n"));
    failure.metadata = json!({
        "attempts": attempts,
        "result_count": 0,
    });
    Ok(failure)
}

fn build_success_result(
    query: &str,
    started_at: Instant,
    results: Vec<SearchResult>,
    body: String,
    attempts: Vec<Value>,
) -> ScrapedResult {
    let mut result = ScrapedResult::success(SourceName::DuckDuckGo, query, started_at);
    result.search_results = results;
    result.raw_html = Some(trim_preview(&body, 2_000));
    result.metadata = json!({
        "attempts": attempts,
        "result_count": result.search_results.len(),
    });
    result
}

fn parse_results(body: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(body);
    let result_selector = Selector::parse(".result").expect("valid selector");
    let title_selector = Selector::parse(".result__title a, .result__a").expect("valid selector");
    let snippet_selector = Selector::parse(".result__snippet").expect("valid selector");

    let mut results = Vec::new();
    for item in document
        .select(&result_selector)
        .take(DUCKDUCKGO_SEARCH_FETCH_LIMIT)
    {
        let Some(title_link) = item.select(&title_selector).next() else {
            continue;
        };
        results.push(SearchResult {
            title: title_link.text().collect::<String>().trim().to_owned(),
            url: title_link
                .value()
                .attr("href")
                .map(normalize_duckduckgo_url)
                .unwrap_or_default(),
            snippet: item
                .select(&snippet_selector)
                .next()
                .map(|node| node.text().collect::<String>().trim().to_owned())
                .unwrap_or_default(),
        });
    }
    results
}

async fn fetch_duckduckgo_html_via_reqwest(query: &str) -> anyhow::Result<String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("vi-VN,vi;q=0.9,en;q=0.8"),
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(REFERER, HeaderValue::from_static("https://duckduckgo.com/"));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(DUCKDUCKGO_DESKTOP_USER_AGENT),
    );

    let client = ReqwestClient::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    Ok(client
        .post("https://html.duckduckgo.com/html/")
        .form(&[("q", query)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

fn detect_duckduckgo_block_kind(body: &str) -> Option<&'static str> {
    let lower = body.to_ascii_lowercase();
    if lower.contains("anomaly-modal")
        || lower.contains("anomaly.js")
        || lower.contains("bots use duckduckgo too")
    {
        return Some("anomaly");
    }
    if lower.contains("captcha") {
        return Some("captcha");
    }
    None
}

fn extract_title(body: &str) -> String {
    let document = Html::parse_document(body);
    let title_selector = Selector::parse("title").expect("valid selector");
    document
        .select(&title_selector)
        .next()
        .map(|node| node.text().collect::<String>().trim().to_owned())
        .unwrap_or_default()
}

fn build_duckduckgo_failure_message(attempts: &[Value]) -> String {
    if attempts.iter().any(|attempt| {
        attempt
            .get("blocked")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "anomaly")
    }) {
        return "duckduckgo returned anomaly challenge".to_owned();
    }

    attempts
        .iter()
        .rev()
        .find_map(|attempt| attempt.get("error").and_then(Value::as_str))
        .filter(|error| !error.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "duckduckgo returned empty results".to_owned())
}

fn trim_preview(body: &str, max_chars: usize) -> String {
    body.chars().take(max_chars).collect()
}

fn normalize_duckduckgo_url(href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_owned();
    }

    let candidates = [
        format!("https:{href}"),
        format!("https://duckduckgo.com{href}"),
    ];

    for candidate in candidates {
        if let Ok(url) = Url::parse(&candidate) {
            if let Some(target) = url
                .query_pairs()
                .find(|(key, _)| key == "uddg")
                .map(|(_, value)| value.to_string())
            {
                return target;
            }
            return url.to_string();
        }
    }

    href.to_owned()
}
