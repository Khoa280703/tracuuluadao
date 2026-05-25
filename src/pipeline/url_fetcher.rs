use std::env;

use scraper::{Html, Selector};
use url::Url;

use crate::error::AppResult;
use crate::scrapers::http_client::{COOKIE, HeaderMap, HeaderValue, HttpClient, REFERER};

#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub url: String,
    pub content: String,
}

pub async fn fetch_page(client: &HttpClient, url: &str) -> AppResult<FetchedPage> {
    let normalized_url = normalize_url(url);
    let html = fetch_html(client, &normalized_url).await?;

    let content = if is_facebook_url(&normalized_url) {
        // Facebook serves JS-heavy shell; DOM text is mostly boot scripts garbage.
        // og:description is the most reliable content source for public posts.
        extract_og_description(&html)
            .or_else(|| {
                let text = extract_text(
                    &html,
                    &["div.story_body_container", "div[data-ft]", "article"],
                );
                (text.len() >= 40 && text.len() < 5_000).then_some(text)
            })
            .unwrap_or_default()
    } else {
        extract_text(&html, &["main", "article", "body"])
    };

    Ok(FetchedPage {
        url: normalized_url,
        content: trim_text(&content, 2_000),
    })
}

async fn fetch_html(client: &HttpClient, normalized_url: &str) -> AppResult<String> {
    if is_facebook_url(normalized_url) {
        if let Some(cookie) = facebook_cookie_header() {
            let mut headers = HeaderMap::new();
            let cookie_value = HeaderValue::from_str(&cookie)
                .map_err(|error| crate::error::AppError::Server(error.to_string()))?;
            headers.insert(COOKIE, cookie_value);
            headers.insert(
                REFERER,
                HeaderValue::from_static("https://www.facebook.com/"),
            );
            return client
                .get_text_with_headers(normalized_url, headers)
                .await
                .map_err(|error| crate::error::AppError::Server(error.to_string()));
        }
    }

    client
        .get_text(normalized_url)
        .await
        .map_err(|error| crate::error::AppError::Server(error.to_string()))
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

fn facebook_cookie_header() -> Option<String> {
    env::var("FACEBOOK_COOKIE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_og_description(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("meta[property=\"og:description\"]").ok()?;
    doc.select(&selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(decode_html_entities)
        .filter(|s| s.len() >= 20)
}

fn decode_html_entities(input: &str) -> String {
    let mut result = input.to_string();
    result = result.replace("&amp;", "&");
    result = result.replace("&lt;", "<");
    result = result.replace("&gt;", ">");
    result = result.replace("&quot;", "\"");
    result = result.replace("&#039;", "'");
    result = result.replace("&apos;", "'");
    // Decode numeric entities (&#xHEX; and &#DEC;)
    while let Some(start) = result.find("&#") {
        let rest = &result[start + 2..];
        let (radix, skip) = if rest.starts_with('x') || rest.starts_with('X') {
            (16, 1)
        } else {
            (10, 0)
        };
        if let Some(end) = rest[skip..].find(';') {
            let num_str = &rest[skip..skip + end];
            if let Ok(code) = u32::from_str_radix(num_str, radix) {
                if let Some(ch) = char::from_u32(code) {
                    let full_len = 2 + skip + end + 1; // &#x...;
                    result = format!("{}{ch}{}", &result[..start], &result[start + full_len..]);
                    continue;
                }
            }
        }
        break;
    }
    result
}

fn extract_text(html: &str, selectors: &[&str]) -> String {
    let document = Html::parse_document(html);
    let mut best = String::new();

    for pattern in selectors {
        let Ok(selector) = Selector::parse(pattern) else {
            continue;
        };
        for node in document.select(&selector) {
            let text = node
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if text.len() > best.len() {
                best = text;
            }
        }
    }

    if best.is_empty() {
        html.to_string()
    } else {
        best
    }
}

fn is_facebook_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .map(|host| {
            let host = host.to_ascii_lowercase();
            host == "facebook.com"
                || host.ends_with(".facebook.com")
                || host == "fb.watch"
                || host.ends_with(".fb.watch")
        })
        .unwrap_or(false)
}

fn trim_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let trimmed = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        trimmed
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{facebook_cookie_header, fetch_html, fetch_page, is_facebook_url};
    use crate::scrapers::http_client::HttpClientFactory;
    use scraper::{Html, Selector};

    const FACEBOOK_DIAGNOSTIC_URLS: [&str; 3] = [
        "https://www.facebook.com/groups/2659887910844175/posts/3354541331378826/",
        "https://www.facebook.com/groups/gropzingspeed.vng/posts/1964790024119693/",
        "https://www.facebook.com/groups/871890707767827/posts/1236268011330093/",
    ];

    #[test]
    fn detects_facebook_hosts() {
        assert!(is_facebook_url(
            "https://www.facebook.com/groups/test/posts/1/"
        ));
        assert!(is_facebook_url("https://facebook.com/page"));
        assert!(!is_facebook_url("https://example.com/post/1"));
    }

    #[test]
    fn reads_facebook_cookie_from_env() {
        unsafe {
            std::env::set_var("FACEBOOK_COOKIE", "c_user=1; xs=abc");
        }
        assert_eq!(
            facebook_cookie_header().as_deref(),
            Some("c_user=1; xs=abc")
        );
        unsafe {
            std::env::remove_var("FACEBOOK_COOKIE");
        }
    }

    #[tokio::test]
    #[ignore = "network diagnostic for live Facebook parsing"]
    async fn fetches_live_facebook_post_with_runtime_cookie_when_available() {
        dotenvy::dotenv().ok();
        let client = HttpClientFactory::default()
            .standard_client()
            .expect("http client");
        let url = "https://www.facebook.com/groups/2659887910844175/posts/3354541331378826/";
        let html = fetch_html(&client, url).await.expect("fetch html");
        let page = fetch_page(&client, url).await.expect("fetch page");

        println!("html_len={}", html.len());
        println!(
            "html_preview={}",
            html.chars()
                .take(500)
                .collect::<String>()
                .replace('\n', " ")
        );
        println!("content_len={}", page.content.len());
        println!("content={}", page.content);
    }

    #[tokio::test]
    #[ignore = "network diagnostic for cookie/no-cookie Facebook fetch comparison"]
    async fn compares_facebook_fetch_with_and_without_cookie() {
        dotenvy::dotenv().ok();
        let original_cookie = std::env::var("FACEBOOK_COOKIE").ok();
        let client = HttpClientFactory::default()
            .standard_client()
            .expect("http client");

        for url in FACEBOOK_DIAGNOSTIC_URLS {
            unsafe {
                std::env::remove_var("FACEBOOK_COOKIE");
            }
            let without_cookie = fetch_page(&client, url).await;

            match &original_cookie {
                Some(cookie) => unsafe {
                    std::env::set_var("FACEBOOK_COOKIE", cookie);
                },
                None => unsafe {
                    std::env::remove_var("FACEBOOK_COOKIE");
                },
            }
            let with_cookie = fetch_page(&client, url).await;
            let html_without_cookie = fetch_html(&client, url).await.ok();

            println!("url={url}");
            if let Some(html) = &html_without_cookie {
                println!("meta={}", html_meta_preview(html));
            }
            match &without_cookie {
                Ok(page) => println!(
                    "without_cookie len={} preview={}",
                    page.content.len(),
                    page.content.chars().take(160).collect::<String>()
                ),
                Err(error) => println!("without_cookie error={error}"),
            }
            match &with_cookie {
                Ok(page) => println!(
                    "with_cookie len={} preview={}",
                    page.content.len(),
                    page.content.chars().take(160).collect::<String>()
                ),
                Err(error) => println!("with_cookie error={error}"),
            }
            println!("---");
        }

        match original_cookie {
            Some(cookie) => unsafe {
                std::env::set_var("FACEBOOK_COOKIE", cookie);
            },
            None => unsafe {
                std::env::remove_var("FACEBOOK_COOKIE");
            },
        }
    }

    fn html_meta_preview(html: &str) -> String {
        let document = Html::parse_document(html);
        let title = Selector::parse("title")
            .ok()
            .and_then(|selector| {
                document
                    .select(&selector)
                    .next()
                    .map(|node| node.text().collect::<String>())
            })
            .unwrap_or_default();
        let og_url = Selector::parse("meta[property=\"og:url\"]")
            .ok()
            .and_then(|selector| {
                document
                    .select(&selector)
                    .next()
                    .and_then(|node| node.value().attr("content"))
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let og_description = Selector::parse("meta[property=\"og:description\"]")
            .ok()
            .and_then(|selector| {
                document
                    .select(&selector)
                    .next()
                    .and_then(|node| node.value().attr("content"))
                    .map(str::to_owned)
            })
            .unwrap_or_default();

        format!(
            "title={title:?} og_url={og_url:?} og_description_preview={:?}",
            og_description.chars().take(160).collect::<String>()
        )
    }
}
