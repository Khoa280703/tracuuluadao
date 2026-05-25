use std::time::Duration;

use anyhow::Result;
use rquest::header::{ACCEPT, ACCEPT_LANGUAGE};
pub use rquest::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT};
use rquest::{Client as InnerClient, Proxy};
use rquest_util::Emulation;
use serde_json::Value;
use url::Url;

use crate::scrapers::proxy::ProxyConfig;

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

/// Desktop Chrome UA variants used when fetching Google search via proxy.
/// Using mobile GSA (NSTNWV) UAs triggers a JS-execution challenge page
/// (`/httpservice/retry/enablejs`) that cannot be resolved without a real browser.
/// Desktop UAs receive plain HTML search results pages that are parseable.
const DESKTOP_USER_AGENTS: [&str; 3] = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
];

#[derive(Debug, Clone, Default)]
pub struct HttpClientFactory;

#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: InnerClient,
}

impl HttpClientFactory {
    pub fn standard_client(&self) -> Result<HttpClient> {
        self.build(DEFAULT_USER_AGENT, None)
    }

    pub fn build_with_proxy(&self, proxy: &ProxyConfig) -> Result<HttpClient> {
        self.build(DEFAULT_USER_AGENT, Some(proxy))
    }

    pub fn google_client(&self, proxy: Option<&ProxyConfig>) -> Result<HttpClient> {
        // Desktop UAs work through proxies; mobile GSA (NSTNWV) UAs trigger a
        // JS-execution challenge (/httpservice/retry/enablejs) that yields 0 results.
        let user_agent = proxy
            .and_then(|proxy| {
                DESKTOP_USER_AGENTS.get(proxy.rotation_seed % DESKTOP_USER_AGENTS.len())
            })
            .copied()
            .unwrap_or(DESKTOP_USER_AGENTS[0]);
        self.build(user_agent, proxy)
    }

    fn build(&self, user_agent: &str, proxy: Option<&ProxyConfig>) -> Result<HttpClient> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        headers.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("vi-VN,vi;q=0.9,en;q=0.8"),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(user_agent).expect("static user agent must be valid"),
        );

        let mut builder = InnerClient::builder()
            .default_headers(headers)
            .cookie_store(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .emulation(Emulation::Chrome136);

        if let Some(proxy) = proxy {
            builder = builder.proxy(Proxy::all(&proxy.url)?);
        }

        Ok(HttpClient {
            inner: builder.build()?,
        })
    }
}

impl HttpClient {
    pub async fn get_text(&self, url: &str) -> Result<String> {
        Ok(self.inner.get(url).send().await?.text().await?)
    }

    pub async fn get_text_with_headers(&self, url: &str, headers: HeaderMap) -> Result<String> {
        Ok(self
            .inner
            .get(url)
            .headers(headers)
            .send()
            .await?
            .text()
            .await?)
    }

    pub async fn get_text_with_query(&self, url: &str, query: &[(&str, String)]) -> Result<String> {
        Ok(self
            .inner
            .get(url)
            .query(query)
            .send()
            .await?
            .text()
            .await?)
    }

    pub async fn get_json_value(&self, url: &str) -> Result<Value> {
        Ok(self.inner.get(url).send().await?.json().await?)
    }

    pub async fn get_cookie_value(&self, url: &str, cookie_name: &str) -> Result<Option<String>> {
        let response = self.inner.get(url).send().await?;
        Ok(response
            .cookies()
            .find(|cookie| cookie.name() == cookie_name)
            .map(|cookie| cookie.value().to_owned()))
    }

    pub async fn post_form_text(
        &self,
        url: &str,
        headers: HeaderMap,
        form: &[(&str, &str)],
    ) -> Result<String> {
        Ok(self
            .inner
            .post(url)
            .headers(headers)
            .form(form)
            .send()
            .await?
            .text()
            .await?)
    }

    pub async fn get_text_from_url(&self, url: Url) -> Result<String> {
        Ok(self.inner.get(url).send().await?.text().await?)
    }
}

/// Detects Cloudflare block pages (403 challenge, JS challenge, CAPTCHA, Turnstile).
pub fn is_cloudflare_blocked(html: &str) -> bool {
    let lower = html.to_lowercase();
    if !lower.contains("cloudflare")
        && !lower.contains("cf-error")
        && !lower.contains("cf-challenge")
    {
        return false;
    }
    lower.contains("you have been blocked")
        || lower.contains("attention required")
        || lower.contains("just a moment")
        || lower.contains("enable cookies")
        || lower.contains("cf-challenge")
        || lower.contains("challenge-platform")
        || lower.contains("ray id")
        || lower.contains("cf-turnstile")
}
