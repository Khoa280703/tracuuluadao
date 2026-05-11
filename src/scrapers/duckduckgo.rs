use std::time::Instant;

use scraper::{Html, Selector};
use url::Url;

use crate::pipeline::QueryType;
use crate::scrapers::http_client::HttpClient;
use crate::scrapers::{ScrapedResult, SearchResult, SourceName};

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
    let body = client
        .post_form_text(
            "https://html.duckduckgo.com/html/",
            Default::default(),
            &[("q", &search_query)],
        )
        .await?;
    let document = Html::parse_document(&body);
    let result_selector = Selector::parse(".result").expect("valid selector");
    let title_selector = Selector::parse(".result__title a, .result__a").expect("valid selector");
    let snippet_selector = Selector::parse(".result__snippet").expect("valid selector");

    let mut results = Vec::new();
    for item in document.select(&result_selector).take(10) {
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

    let mut result = ScrapedResult::success(SourceName::DuckDuckGo, query, started_at);
    result.search_results = results;
    result.raw_html = Some(body.chars().take(2_000).collect());
    result.success = !result.search_results.is_empty();
    Ok(result)
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
