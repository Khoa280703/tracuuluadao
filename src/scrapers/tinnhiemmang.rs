use std::time::Instant;

use scraper::{Html, Selector};
use serde_json::json;

use crate::scrapers::http_client::{CONTENT_TYPE, HeaderMap, HeaderValue, HttpClient, REFERER};
use crate::scrapers::{ScrapedReport, ScrapedResult, SourceName};

pub async fn scrape(client: Option<HttpClient>, query: &str) -> ScrapedResult {
    let started_at = Instant::now();
    let Some(client) = client else {
        return ScrapedResult::failure(
            SourceName::TinNhiemMang,
            query,
            started_at,
            "http client unavailable",
        );
    };

    match scrape_impl(&client, query, started_at).await {
        Ok(result) => result,
        Err(error) => ScrapedResult::failure(
            SourceName::TinNhiemMang,
            query,
            started_at,
            error.to_string(),
        ),
    }
}

async fn scrape_impl(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let xsrf = client
        .get_cookie_value("https://tinnhiemmang.vn", "XSRF-TOKEN")
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing XSRF-TOKEN"))?;

    let mut headers = HeaderMap::new();
    headers.insert("X-XSRF-TOKEN", HeaderValue::from_str(&xsrf)?);
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://tinnhiemmang.vn/"),
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );

    let html = client
        .post_form_text(
            "https://tinnhiemmang.vn/searchOrg",
            headers,
            &[("search", query)],
        )
        .await?;

    let document = Html::parse_document(&html);
    let item_selector = Selector::parse("li, a, tr, .result-item").expect("valid selector");
    let mut reports = Vec::new();
    for item in document.select(&item_selector).take(10) {
        let text = item.text().collect::<String>().trim().to_owned();
        if text.len() <= 3 {
            continue;
        }
        reports.push(ScrapedReport {
            title: text.clone(),
            url: item.value().attr("href").unwrap_or_default().to_owned(),
            content: text,
            date: None,
        });
    }

    let mut result = ScrapedResult::success(SourceName::TinNhiemMang, query, started_at);
    result.reports = reports;
    result.metadata = json!({ "result_count": result.reports.len() });
    result.raw_html = Some(html.chars().take(2_000).collect());
    Ok(result)
}
