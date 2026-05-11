use scraper::{Html, Selector};
use url::Url;

use crate::error::AppResult;
use crate::scrapers::http_client::HttpClient;

#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub url: String,
    pub content: String,
}

pub async fn fetch_page(client: &HttpClient, url: &str) -> AppResult<FetchedPage> {
    let normalized_url = normalize_url(url);
    let html = client
        .get_text(&normalized_url)
        .await
        .map_err(|error| crate::error::AppError::Server(error.to_string()))?;
    let document = Html::parse_document(&html);
    let selector = Selector::parse("main, article, body").expect("valid selector");
    let content = document
        .select(&selector)
        .next()
        .map(|node| node.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| html.clone());

    Ok(FetchedPage {
        url: normalized_url,
        content: content.chars().take(2_000).collect(),
    })
}

fn normalize_url(url: &str) -> String {
    if Url::parse(url).is_ok() {
        return url.to_string();
    }

    let prefixed = format!("https://{}", url.trim_start_matches('/'));
    if Url::parse(&prefixed).is_ok() {
        return prefixed;
    }

    url.to_string()
}
