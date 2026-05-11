use std::time::Instant;

use regex::Regex;
use scraper::{Html, Selector};
use serde_json::json;

use crate::scrapers::http_client::HttpClient;
use crate::scrapers::{ScrapedReport, ScrapedResult, SourceName};

pub async fn scrape(client: Option<HttpClient>, query: &str) -> ScrapedResult {
    let started_at = Instant::now();
    let Some(client) = client else {
        return ScrapedResult::failure(
            SourceName::TrangTrang,
            query,
            started_at,
            "http client unavailable",
        );
    };

    match scrape_impl(&client, query, started_at).await {
        Ok(result) => result,
        Err(error) => {
            ScrapedResult::failure(SourceName::TrangTrang, query, started_at, error.to_string())
        }
    }
}

async fn scrape_impl(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let url = format!("https://trangtrang.com/{query}");
    let html = client.get_text(&url).await?;
    let document = Html::parse_document(&html);
    let main_selector = Selector::parse("main, article, body").expect("valid selector");
    let comment_selector =
        Selector::parse("[class*='comment'], [class*='review'], [class*='feedback']")
            .expect("valid selector");

    let text = document
        .select(&main_selector)
        .next()
        .map(|node| node.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let carrier_regex =
        Regex::new(r"(Viettel|Mobifone|MobiFone|Vinaphone|Vietnamobile|Gmobile|Reddi)")?;
    let carrier = carrier_regex
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| "Unknown".to_owned());

    let comments = document
        .select(&comment_selector)
        .map(|node| node.text().collect::<String>().trim().to_owned())
        .filter(|text| text.len() > 10)
        .take(5)
        .collect::<Vec<_>>();

    let mut result = ScrapedResult::success(SourceName::TrangTrang, query, started_at);
    result.reports = vec![ScrapedReport {
        title: format!("trangtrang:{query}"),
        url: format!("https://trangtrang.com/{query}"),
        content: text.clone(),
        date: None,
    }];
    result.metadata = json!({ "carrier": carrier, "comments": comments });
    result.raw_html = Some(html.chars().take(2_000).collect());
    Ok(result)
}
