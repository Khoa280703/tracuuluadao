use std::collections::HashSet;
use std::time::Instant;

use futures::stream::{self, StreamExt};
use regex::Regex;
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use crate::scrapers::http_client::{HttpClient, HttpClientFactory, is_cloudflare_blocked};
use crate::scrapers::proxy::ProxyPool;
use crate::scrapers::{ScrapedReport, ScrapedResult, SourceName};

const STATIC_SLUGS: &[&str] = &[
    "thong-tin",
    "lien-he",
    "marketing",
    "gioi-thieu",
    "bio",
    "to-cao-lua-dao",
    "chinh-sach-bao-mat",
    "dieu-khoan-su-dung",
    "huong-dan",
    "lien-ket",
    "tin-tuc",
    "hoi-dap",
];
const CHECKSCAM_REPORT_FETCH_LIMIT: usize = 100;
const CHECKSCAM_SIDECAR_DETAIL_LIMIT: usize = 20;
const CHECKSCAM_REST_PAGE_SIZE: usize = 100;
const CHECKSCAM_REST_MAX_PAGES: usize = 4;
const CHECKSCAM_DETAIL_CONCURRENCY: usize = 4;
const CHECKSCAM_REST_CONCURRENCY: usize = 6;
pub const SIDECAR_BASE_URL: &str = "http://127.0.0.1:4417";

#[derive(Debug, Deserialize)]
struct RestRenderedText {
    rendered: String,
}

#[derive(Debug, Deserialize)]
struct RestPostRecord {
    link: String,
    title: RestRenderedText,
}

pub async fn scrape(
    factory: HttpClientFactory,
    proxy_pool: Option<&ProxyPool>,
    query: &str,
) -> ScrapedResult {
    let started_at = Instant::now();
    let Ok(client) = factory.standard_client() else {
        return ScrapedResult::failure(
            SourceName::CheckScam,
            query,
            started_at,
            "http client unavailable",
        );
    };

    // Strategy 1: browser sidecar (bypasses Cloudflare via warm browser session)
    match scrape_via_sidecar(&client, query, started_at).await {
        Ok(result) => return result,
        Err(error) => {
            tracing::debug!(query = %query, error = %error, "checkscam sidecar unavailable, falling back to direct");
        }
    }

    // Strategy 2: direct rquest (works when Cloudflare is not active)
    match scrape_impl(&client, query, started_at).await {
        Ok(result) => return result,
        Err(error) => {
            let msg = error.to_string();
            if !msg.contains("cloudflare") {
                return ScrapedResult::failure(SourceName::CheckScam, query, started_at, msg);
            }
            tracing::info!(query = %query, "checkscam direct blocked by cloudflare, trying proxy");
        }
    }

    // Strategy 3: proxy (last resort)
    let Some(proxy) = proxy_pool.and_then(|pool| pool.pick()) else {
        return ScrapedResult::failure(
            SourceName::CheckScam,
            query,
            started_at,
            "cloudflare blocked, sidecar unavailable, no proxy",
        );
    };
    let Ok(proxy_client) = factory.build_with_proxy(proxy) else {
        return ScrapedResult::failure(
            SourceName::CheckScam,
            query,
            started_at,
            "cloudflare blocked, proxy client build failed",
        );
    };
    match scrape_impl(&proxy_client, query, started_at).await {
        Ok(result) => result,
        Err(error) => ScrapedResult::failure(
            SourceName::CheckScam,
            query,
            started_at,
            format!("all strategies failed: {error}"),
        ),
    }
}

async fn scrape_via_sidecar(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let sidecar_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(35))
        .build()?;

    let resp = sidecar_client
        .post(format!("{SIDECAR_BASE_URL}/checkscam"))
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    if !body["ok"].as_bool().unwrap_or(false) {
        anyhow::bail!(
            "sidecar error: {}",
            body["error"].as_str().unwrap_or("unknown")
        );
    }

    let html = body["html"].as_str().unwrap_or("").to_owned();
    if html.is_empty() {
        anyhow::bail!("sidecar returned empty html");
    }

    tracing::info!(query = %query, html_len = html.len(), "checkscam fetched via browser sidecar");
    scrape_from_html(client, query, &html, started_at, Some(&sidecar_client)).await
}

async fn scrape_impl(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let url = build_search_url(query)?;
    let html = client.get_text_from_url(url).await?;

    if is_cloudflare_blocked(&html) {
        anyhow::bail!("cloudflare blocked checkscam request");
    }

    scrape_from_html(client, query, &html, started_at, None).await
}

async fn scrape_from_html(
    client: &HttpClient,
    query: &str,
    html: &str,
    started_at: Instant,
    sidecar_client: Option<&reqwest::Client>,
) -> anyhow::Result<ScrapedResult> {
    let count_regex = Regex::new(r"Có\s+(\d+)\s+cảnh báo")?;

    let report_count = count_regex
        .captures(&html)
        .and_then(|captures| captures.get(1))
        .and_then(|count| count.as_str().parse::<u64>().ok())
        .unwrap_or_default();

    let search_results = collect_search_results(&html);
    let direct_links = search_results
        .iter()
        .map(|item| item.href.clone())
        .collect::<Vec<_>>();
    let rest_candidates = if sidecar_client.is_some() {
        Vec::new()
    } else {
        collect_rest_candidates(client, &direct_links)
            .await
            .unwrap_or_default()
    };

    let mut candidate_urls = Vec::new();
    let mut seen_urls = HashSet::new();
    for url in direct_links.into_iter().chain(rest_candidates.into_iter()) {
        if seen_urls.insert(url.clone()) {
            candidate_urls.push(url);
        }
    }

    let slugs = {
        let document = Html::parse_document(&html);
        let anchor_selector = Selector::parse("a[href]").expect("valid selector");
        let mut seen = HashSet::new();
        let mut slugs = Vec::new();

        for element in document.select(&anchor_selector) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            let Some(slug) = href.split("checkscam.vn/").nth(1) else {
                continue;
            };
            let slug = slug.trim_matches('/').trim();
            if slug.is_empty()
                || slug.contains('?')
                || slug.contains('/')
                || slug.starts_with("wp-")
                || STATIC_SLUGS.contains(&slug)
                || !seen.insert(slug.to_owned())
            {
                continue;
            }
            slugs.push((
                slug.to_owned(),
                href.to_owned(),
                element.text().collect::<String>().trim().to_owned(),
            ));
        }

        slugs
    };

    let mut reports = Vec::new();
    let mut report_urls_seen = HashSet::new();
    let detail_limit = if sidecar_client.is_some() {
        CHECKSCAM_SIDECAR_DETAIL_LIMIT
    } else {
        CHECKSCAM_REPORT_FETCH_LIMIT
    };
    let detail_results = stream::iter(candidate_urls.into_iter().take(detail_limit))
        .map(|detail_url| {
            let client = client.clone();
            let sidecar = sidecar_client.cloned();
            async move {
                let detail_html = fetch_detail_html(&client, sidecar.as_ref(), &detail_url).await?;
                Ok::<_, anyhow::Error>(parse_detail_report(&detail_url, "", &detail_html))
            }
        })
        .buffer_unordered(CHECKSCAM_DETAIL_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    for result in detail_results {
        let Ok(report) = result else { continue };
        if !detail_matches_query(&report, query) {
            continue;
        }
        if report_urls_seen.insert(report.url.clone()) {
            reports.push(report);
        }
    }

    if reports.is_empty() {
        for (slug, href, title) in slugs.into_iter().take(detail_limit) {
            let detail_url = if href.starts_with("http") {
                href
            } else {
                format!("https://checkscam.vn/{slug}/")
            };
            let detail_html = match fetch_detail_html(client, sidecar_client, &detail_url).await {
                Ok(html) => html,
                Err(_) => continue,
            };
            let report = parse_detail_report(&detail_url, &title, &detail_html);
            if report_urls_seen.insert(report.url.clone()) {
                reports.push(report);
            }
        }
    }

    let mut result = ScrapedResult::success(SourceName::CheckScam, query, started_at);
    result.reports = reports;
    result.raw_html = Some(html.chars().take(2_000).collect());
    result.metadata = json!({ "report_count": report_count });
    Ok(result)
}

async fn fetch_detail_html(
    client: &HttpClient,
    sidecar_client: Option<&reqwest::Client>,
    detail_url: &str,
) -> anyhow::Result<String> {
    if let Some(sidecar) = sidecar_client {
        match fetch_detail_via_sidecar(sidecar, detail_url).await {
            Ok(html) => return Ok(html),
            Err(error) => {
                tracing::debug!(url = %detail_url, error = %error, "checkscam detail sidecar failed, trying direct");
            }
        }
    }
    Ok(client.get_text(detail_url).await?)
}

async fn fetch_detail_via_sidecar(
    sidecar_client: &reqwest::Client,
    detail_url: &str,
) -> anyhow::Result<String> {
    let resp = sidecar_client
        .post(format!("{SIDECAR_BASE_URL}/checkscam/detail"))
        .json(&serde_json::json!({ "url": detail_url }))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;
    if !body["ok"].as_bool().unwrap_or(false) {
        anyhow::bail!(
            "sidecar detail error: {}",
            body["error"].as_str().unwrap_or("unknown")
        );
    }
    let html = body["html"].as_str().unwrap_or("").to_owned();
    if html.is_empty() {
        anyhow::bail!("sidecar returned empty detail html");
    }
    Ok(html)
}

fn build_search_url(query: &str) -> anyhow::Result<url::Url> {
    url::Url::parse_with_params("https://checkscam.vn/", &[("qh_ss", query)]).map_err(Into::into)
}

fn collect_search_results(html: &str) -> Vec<SearchResultLink> {
    let document = Html::parse_document(html);
    let pst_selector = Selector::parse(".pst").expect("valid selector");
    let link_selector = Selector::parse(".ct > .ct1 > a[href]").expect("valid selector");
    let mut seen = HashSet::new();
    let mut results = Vec::new();

    if let Some(first_pst) = document.select(&pst_selector).next() {
        for link in first_pst.select(&link_selector) {
            let Some(href) = link.value().attr("href") else {
                continue;
            };
            if href.contains("qh_ss=") || !href.contains("checkscam.vn/") {
                continue;
            }
            if !seen.insert(href.to_owned()) {
                continue;
            }
            results.push(SearchResultLink {
                href: href.to_owned(),
            });
        }
    }

    results
}

#[derive(Debug)]
struct SearchResultLink {
    href: String,
}

async fn collect_rest_candidates(
    client: &HttpClient,
    direct_links: &[String],
) -> anyhow::Result<Vec<String>> {
    let mut candidate_urls = Vec::new();
    let mut seen = HashSet::new();
    let mut titles = HashSet::new();

    for link in direct_links {
        if seen.insert(link.clone()) {
            candidate_urls.push(link.clone());
        }
    }

    let slug_results = stream::iter(direct_links.iter().cloned())
        .map(|link| {
            let client = client.clone();
            async move { fetch_rest_post_by_slug(&client, &link).await }
        })
        .buffer_unordered(CHECKSCAM_REST_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    for result in slug_results {
        if let Some(record) = result? {
            let title = record.title.rendered.trim();
            if !title.is_empty() {
                titles.insert(title.to_owned());
            }
        }
    }

    for title in titles {
        for record in fetch_rest_posts_by_title(client, &title).await? {
            if seen.insert(record.link.clone()) {
                candidate_urls.push(record.link);
            }
        }
    }

    candidate_urls.retain(|url| {
        let normalized = url.to_lowercase();
        normalized.contains("checkscam.vn/")
            && !normalized.contains("/bio/")
            && !normalized.contains("/newfeed/")
            && !normalized.contains("/dich-vu-uy-tin/")
            && !normalized.contains("qh_ss=")
            && normalized
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .is_some_and(|slug| !slug.is_empty() && slug != "checkscam.vn")
    });

    Ok(candidate_urls)
}

async fn fetch_rest_post_by_slug(
    client: &HttpClient,
    detail_url: &str,
) -> anyhow::Result<Option<RestPostRecord>> {
    let Some(slug) = detail_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let url = Url::parse_with_params(
        "https://checkscam.vn/wp-json/wp/v2/posts",
        &[("slug", slug)],
    )?;
    let body = client.get_text_from_url(url).await?;
    let mut records = serde_json::from_str::<Vec<RestPostRecord>>(&body)?;
    Ok(records.pop())
}

async fn fetch_rest_posts_by_title(
    client: &HttpClient,
    title: &str,
) -> anyhow::Result<Vec<RestPostRecord>> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for page in 1..=CHECKSCAM_REST_MAX_PAGES {
        let url = Url::parse_with_params(
            "https://checkscam.vn/wp-json/wp/v2/posts",
            &[
                ("search", title),
                ("per_page", &CHECKSCAM_REST_PAGE_SIZE.to_string()),
                ("page", &page.to_string()),
            ],
        )?;
        let body = client.get_text_from_url(url).await?;
        let page_records = serde_json::from_str::<Vec<RestPostRecord>>(&body)?;
        if page_records.is_empty() {
            break;
        }

        let page_len = page_records.len();
        for record in page_records {
            if seen.insert(record.link.clone()) {
                results.push(record);
            }
        }

        if page_len < CHECKSCAM_REST_PAGE_SIZE {
            break;
        }
    }

    Ok(results)
}

fn detail_matches_query(report: &ScrapedReport, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }

    report.url.to_lowercase().contains(&needle)
        || report.title.to_lowercase().contains(&needle)
        || report.content.to_lowercase().contains(&needle)
}

pub fn parse_detail_report(url: &str, fallback_title: &str, html: &str) -> ScrapedReport {
    let document = Html::parse_document(html);
    let title_selector = Selector::parse("h1, h2").expect("valid selector");
    let history_selector =
        Selector::parse(".sclq .b .c a, .sclq2 .b .c a").expect("valid selector");
    let date_selector = Selector::parse("time, .entry-date, .meta-date").expect("valid selector");
    let collapsed_text = document.root_element().text().collect::<Vec<_>>().join(" ");
    let collapsed_text = collapsed_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let quick_summary_regex = Regex::new(
        r"Họ Tên:\s*(.*?)\s*,\s*SĐT:\s*(.*?)\s*,\s*STK:\s*([0-9*]+)\s*,\s*Ngân hàng:\s*(.*?)\s*,\s*(.*?)(?:#s1|KIỂM TRA & TỐ CÁO SCAM)",
    )
    .expect("valid regex");
    let owner_regex = Regex::new(r"Chủ tk:\s*(.*?)\s*STK:").expect("valid owner regex");
    let phone_regex = Regex::new(r"SĐT:\s*([0-9 .-]{9,16})").expect("valid phone regex");
    let account_regex = Regex::new(r"STK:\s*([0-9*]{6,})").expect("valid account regex");
    let bank_regex =
        Regex::new(r"Ngân hàng:\s*(.*?)\s*(?:Hạng mục:|Ảnh chụp bằng chứng:|Nội dung cảnh báo:)")
            .expect("valid bank regex");
    let warning_regex = Regex::new(
        r"Nội dung cảnh báo:\s*(.*?)\s*(?:💬|_{3,}|Bình luận Copy link|LỊCH SỬ PHẢN ÁNH)",
    )
    .expect("valid warning regex");
    let report_count_regex =
        Regex::new(r"đã bị cảnh báo\s+(\d+)\s+lần").expect("valid report count regex");

    let heading_title = document
        .select(&title_selector)
        .next()
        .map(|node| node.text().collect::<String>().trim().to_owned())
        .filter(|value| !value.is_empty() && value != "Thông tin cảnh báo:");
    let quick_summary = quick_summary_regex.captures(&collapsed_text);
    let owner = quick_summary
        .as_ref()
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().trim().trim_matches('.').to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            owner_regex
                .captures(&collapsed_text)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().trim().trim_matches('.').to_owned())
                .filter(|value| !value.is_empty())
        });
    let phone = quick_summary
        .as_ref()
        .and_then(|captures| captures.get(2))
        .map(|value| value.as_str().trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            phone_regex
                .captures(&collapsed_text)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().trim().to_owned())
                .filter(|value| !value.is_empty())
        });
    let account = quick_summary
        .as_ref()
        .and_then(|captures| captures.get(3))
        .map(|value| value.as_str().trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            account_regex
                .captures(&collapsed_text)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().trim().to_owned())
                .filter(|value| !value.is_empty())
        });
    let bank = quick_summary
        .as_ref()
        .and_then(|captures| captures.get(4))
        .map(|value| value.as_str().trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            bank_regex
                .captures(&collapsed_text)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().trim().to_owned())
                .filter(|value| !value.is_empty())
        });
    let warning = quick_summary
        .as_ref()
        .and_then(|captures| captures.get(5))
        .map(|value| value.as_str().trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            warning_regex
                .captures(&collapsed_text)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().trim().to_owned())
                .filter(|value| !value.is_empty())
        });
    let report_count = report_count_regex
        .captures(&collapsed_text)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_owned());
    let history_titles = document
        .select(&history_selector)
        .filter_map(|node| {
            let text = node.text().collect::<String>().trim().to_owned();
            (!text.is_empty() && text != "Xem thêm").then_some(text)
        })
        .take(6)
        .collect::<Vec<_>>();

    let title = heading_title
        .or_else(
            || match (owner.as_deref(), account.as_deref(), bank.as_deref()) {
                (Some(owner), Some(account), Some(bank)) => {
                    Some(format!("{owner} - {account} - {bank}"))
                }
                (Some(owner), Some(account), None) => Some(format!("{owner} - {account}")),
                _ => None,
            },
        )
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_title.to_owned());
    let mut content_parts = Vec::new();
    if let Some(owner) = owner {
        content_parts.push(format!("Chủ tài khoản: {owner}"));
    }
    if let Some(phone) = phone {
        content_parts.push(format!("Số điện thoại: {phone}"));
    }
    if let Some(account) = account {
        content_parts.push(format!("Số tài khoản: {account}"));
    }
    if let Some(bank) = bank {
        content_parts.push(format!("Ngân hàng: {bank}"));
    }
    if let Some(warning) = warning {
        let warning = strip_checkscam_noise(&warning);
        if !warning.is_empty() {
            content_parts.push(format!("Nội dung cảnh báo: {warning}"));
        }
    }
    if let Some(report_count) = report_count {
        content_parts.push(format!("Số lần bị cảnh báo trên CheckScam: {report_count}"));
    }
    if !history_titles.is_empty() {
        content_parts.push(format!(
            "Lịch sử phản ánh cùng STK: {}",
            history_titles.join("; ")
        ));
    }
    let content = if content_parts.is_empty() {
        collapsed_text
            .split("#s1")
            .next()
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    } else {
        content_parts.join("\n")
    };
    let date = document
        .select(&date_selector)
        .next()
        .map(|node| node.text().collect::<String>().trim().to_owned())
        .filter(|value| !value.is_empty());

    ScrapedReport {
        title,
        url: url.to_owned(),
        content,
        date,
    }
}

fn strip_checkscam_noise(value: &str) -> String {
    let mut cleaned = value.trim().to_owned();
    for marker in [
        "if(window.location.hostname",
        "{\"@context\"",
        "{ \"@context\"",
        "<script",
        ".wp-block-button__link",
        "body{--wp--preset",
        "window._nslDOMReady",
        "/*! This file is auto-generated */",
    ] {
        if let Some((head, _)) = cleaned.split_once(marker) {
            cleaned = head.trim().to_owned();
        }
    }

    cleaned
        .trim_matches(|ch: char| ch.is_whitespace() || ch == ',' || ch == '.')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::build_search_url;

    #[test]
    fn search_url_preserves_full_query_value() {
        let url = build_search_url("https://evil.example/path?a=1&b=2").expect("url should build");
        let value = url
            .query_pairs()
            .find_map(|(key, value)| (key == "qh_ss").then_some(value.into_owned()))
            .expect("qh_ss should exist");

        assert_eq!(value, "https://evil.example/path?a=1&b=2");
    }
}
