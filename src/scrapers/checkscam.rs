use std::collections::HashSet;
use std::time::Instant;

use regex::Regex;
use scraper::{Html, Selector};
use serde_json::json;

use crate::scrapers::http_client::HttpClient;
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

pub async fn scrape(client: Option<HttpClient>, query: &str) -> ScrapedResult {
    let started_at = Instant::now();
    let Some(client) = client else {
        return ScrapedResult::failure(
            SourceName::CheckScam,
            query,
            started_at,
            "http client unavailable",
        );
    };

    match scrape_impl(&client, query, started_at).await {
        Ok(result) => result,
        Err(error) => {
            ScrapedResult::failure(SourceName::CheckScam, query, started_at, error.to_string())
        }
    }
}

async fn scrape_impl(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let url = build_search_url(query)?;
    let html = client.get_text_from_url(url).await?;
    let count_regex = Regex::new(r"Có\s+(\d+)\s+cảnh báo")?;

    let report_count = count_regex
        .captures(&html)
        .and_then(|captures| captures.get(1))
        .and_then(|count| count.as_str().parse::<u64>().ok())
        .unwrap_or_default();

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
    for (slug, href, title) in slugs.into_iter().take(10) {
        let detail_url = if href.starts_with("http") {
            href
        } else {
            format!("https://checkscam.vn/{slug}/")
        };
        let detail_html = client.get_text(&detail_url).await?;
        reports.push(parse_detail_report(&detail_url, &title, &detail_html));
    }

    let mut result = ScrapedResult::success(SourceName::CheckScam, query, started_at);
    result.reports = reports;
    result.raw_html = Some(html.chars().take(2_000).collect());
    result.metadata = json!({ "report_count": report_count });
    Ok(result)
}

fn build_search_url(query: &str) -> anyhow::Result<url::Url> {
    url::Url::parse_with_params("https://checkscam.vn/", &[("qh_ss", query)]).map_err(Into::into)
}

fn parse_detail_report(url: &str, fallback_title: &str, html: &str) -> ScrapedReport {
    let document = Html::parse_document(html);
    let title_selector = Selector::parse("h1, h2").expect("valid selector");
    let content_selector =
        Selector::parse("article, main, .entry-content, .td-post-content").expect("valid selector");
    let date_selector = Selector::parse("time, .entry-date, .meta-date").expect("valid selector");

    let title = document
        .select(&title_selector)
        .next()
        .map(|node| node.text().collect::<String>().trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_title.to_owned());
    let content = document
        .select(&content_selector)
        .next()
        .map(|node| node.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
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
