use std::time::Duration;

use anyhow::Result;
use serde_json::Value;
use url::Url;

#[cfg(not(feature = "tls-impersonation"))]
pub use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, REFERER};
#[cfg(feature = "tls-impersonation")]
pub use rquest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, REFERER};

#[cfg(not(feature = "tls-impersonation"))]
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
#[cfg(not(feature = "tls-impersonation"))]
use reqwest::{Client as InnerClient, Proxy};
#[cfg(feature = "tls-impersonation")]
use rquest::header::{ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
#[cfg(feature = "tls-impersonation")]
use rquest::{Client as InnerClient, Proxy};
#[cfg(feature = "tls-impersonation")]
use rquest_util::Emulation;

use crate::scrapers::proxy::ProxyConfig;

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

const GSA_USER_AGENTS: [&str; 3] = [
    "Mozilla/5.0 (Linux; Android 12; SM-S901U) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/99.0.4844.88 Mobile Safari/537.36 NSTNWV",
    "Mozilla/5.0 (Linux; Android 11; KFTUWI) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.7680.165 Safari/537.36 NSTNWV",
    "Mozilla/5.0 (Linux; Android 5.0; SM-G900P Build/LRX21T) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/39.0.1005.1041 Mobile Safari/537.36 NSTNWV",
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

    pub fn google_client(&self, proxy: Option<&ProxyConfig>) -> Result<HttpClient> {
        let user_agent = proxy
            .and_then(|proxy| GSA_USER_AGENTS.get(proxy.rotation_seed % GSA_USER_AGENTS.len()))
            .copied()
            .unwrap_or(GSA_USER_AGENTS[0]);
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
            .pool_idle_timeout(Duration::from_secs(30));

        #[cfg(feature = "tls-impersonation")]
        {
            builder = builder.emulation(Emulation::Chrome136);
        }

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
