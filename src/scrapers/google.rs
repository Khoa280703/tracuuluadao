use std::time::Instant;

use scraper::{Html, Selector};
use serde_json::{Value, json};
use url::Url;

use crate::pipeline::QueryType;
use crate::scrapers::http_client::{HttpClient, HttpClientFactory};
use crate::scrapers::proxy::ProxyPool;
use crate::scrapers::{ScrapedResult, SearchResult, SourceName};

pub async fn scrape(
    factory: HttpClientFactory,
    query: &str,
    query_type: QueryType,
    proxy_pool: Option<&ProxyPool>,
) -> ScrapedResult {
    let started_at = Instant::now();
    let first_proxy = proxy_pool.and_then(ProxyPool::pick).cloned();
    let second_proxy = proxy_pool.and_then(ProxyPool::pick).cloned();

    for proxy in [first_proxy.as_ref(), second_proxy.as_ref()] {
        let client = match factory.google_client(proxy) {
            Ok(client) => client,
            Err(error) => {
                return ScrapedResult::failure(
                    SourceName::Google,
                    query,
                    started_at,
                    error.to_string(),
                );
            }
        };

        match scrape_once(&client, query, query_type, started_at).await {
            Ok(result) if result.success && !result.search_results.is_empty() => return result,
            Ok(_) => continue,
            Err(_) => continue,
        }
    }

    ScrapedResult::failure(
        SourceName::Google,
        query,
        started_at,
        "google search returned no usable results",
    )
}

async fn scrape_once(
    client: &HttpClient,
    query: &str,
    query_type: QueryType,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let search_query = decorate_query(query, query_type);
    let body = client
        .get_text_with_query(
            "https://www.google.com/search",
            &[
                ("q", search_query),
                ("hl", "vi".to_owned()),
                ("gl", "vn".to_owned()),
                ("tch", "1".to_owned()),
            ],
        )
        .await?;
    let search_results = parse_google_tch1(&body);
    let mut result = ScrapedResult::success(SourceName::Google, query, started_at);
    result.search_results = search_results;
    result.metadata = json!({ "captcha": body.to_ascii_lowercase().contains("captcha") });
    result.raw_html = Some(body.chars().take(2_000).collect());
    result.success =
        !result.search_results.is_empty() && result.metadata["captcha"] != Value::Bool(true);
    Ok(result)
}

fn decorate_query(query: &str, query_type: QueryType) -> String {
    match query_type {
        QueryType::Phone => format!("{query} lừa đảo"),
        QueryType::Bank => format!("\"{query}\" tài khoản ngân hàng lừa đảo"),
        QueryType::Url => format!("\"{query}\" cảnh báo lừa đảo"),
    }
}

fn parse_google_tch1(body: &str) -> Vec<SearchResult> {
    let fragments = serde_json::Deserializer::from_str(body)
        .into_iter::<Value>()
        .filter_map(Result::ok)
        .filter_map(|value| value.get("d").and_then(Value::as_str).map(str::to_owned))
        .collect::<Vec<_>>();
    parse_html_fragments(&fragments.join(""))
}

fn parse_html_fragments(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_fragment(html);
    let block_selector = Selector::parse("div.g").expect("valid selector");
    let title_selector = Selector::parse("h3").expect("valid selector");
    let link_selector = Selector::parse("a[href]").expect("valid selector");
    let snippet_selector =
        Selector::parse("div.VwiC3b, div[data-sncf='1'], span.aCOpRe, div.s3v9rd, div.yXK7lf")
            .expect("valid selector");

    document
        .select(&block_selector)
        .filter_map(|node| {
            let title = node
                .select(&title_selector)
                .next()?
                .text()
                .collect::<String>()
                .trim()
                .to_owned();
            let href = node
                .select(&link_selector)
                .next()?
                .value()
                .attr("href")?
                .to_owned();
            Some(SearchResult {
                title,
                url: normalize_google_url(&href),
                snippet: node
                    .select(&snippet_selector)
                    .next()
                    .map(|item| item.text().collect::<String>().trim().to_owned())
                    .unwrap_or_default(),
            })
        })
        .take(10)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{normalize_google_url, parse_html_fragments};

    #[test]
    fn parse_google_html_fragments_extracts_snippets() {
        let html = r#"
            <div class="g">
                <a href="/url?q=https://example.com/report&sa=U&ved=2ah">
                    <h3>Cảnh báo giao dịch</h3>
                </a>
                <div class="VwiC3b">Nội dung snippet thử nghiệm về lừa đảo.</div>
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
    fn normalize_google_url_decodes_real_target() {
        let href = "/url?q=https://example.com/path%3Fa%3D1%26b%3D2&sa=U&ved=2ah";
        assert_eq!(
            normalize_google_url(href),
            "https://example.com/path?a=1&b=2"
        );
    }
}
