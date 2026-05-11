pub mod checkscam;
pub mod chongluadao;
pub mod duckduckgo;
pub mod google;
pub mod http_client;
pub mod proxy;
pub mod tinnhiemmang;
pub mod trangtrang;

use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cache::CacheService;
use crate::pipeline::QueryType;
use crate::scrapers::http_client::HttpClientFactory;
use crate::scrapers::proxy::ProxyPool;

const SCRAPER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SourceName {
    CheckScam,
    ChongLuaDao,
    TrangTrang,
    TinNhiemMang,
    Google,
    DuckDuckGo,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrapedReport {
    pub title: String,
    pub url: String,
    pub content: String,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedResult {
    pub source: SourceName,
    pub query: String,
    pub success: bool,
    pub reports: Vec<ScrapedReport>,
    pub search_results: Vec<SearchResult>,
    pub metadata: Value,
    pub raw_html: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

impl ScrapedResult {
    pub fn success(source: SourceName, query: &str, started_at: Instant) -> Self {
        Self {
            source,
            query: query.to_owned(),
            success: true,
            reports: Vec::new(),
            search_results: Vec::new(),
            metadata: Value::Null,
            raw_html: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            error: None,
        }
    }

    pub fn failure(
        source: SourceName,
        query: &str,
        started_at: Instant,
        error: impl Into<String>,
    ) -> Self {
        Self {
            source,
            query: query.to_owned(),
            success: false,
            reports: Vec::new(),
            search_results: Vec::new(),
            metadata: Value::Null,
            raw_html: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            error: Some(error.into()),
        }
    }
}

pub async fn run_all_scrapers(
    query: &str,
    query_type: QueryType,
    proxy_pool: Option<&ProxyPool>,
    cache: Option<&CacheService>,
) -> Vec<ScrapedResult> {
    let factory = HttpClientFactory::default();
    let base_client = factory.standard_client().ok();
    let mut tasks: Vec<BoxFuture<'_, ScrapedResult>> = Vec::new();

    if matches!(
        query_type,
        QueryType::Phone | QueryType::Bank | QueryType::Url
    ) {
        tasks.push(Box::pin(run_cached_scraper(
            SourceName::CheckScam,
            query,
            query_type,
            cache,
            checkscam::scrape(factory.standard_client().ok(), query),
        )));
        tasks.push(Box::pin(run_cached_scraper(
            SourceName::TinNhiemMang,
            query,
            query_type,
            cache,
            tinnhiemmang::scrape(factory.standard_client().ok(), query),
        )));
    }

    if matches!(query_type, QueryType::Phone) {
        tasks.push(Box::pin(run_cached_scraper(
            SourceName::ChongLuaDao,
            query,
            query_type,
            cache,
            chongluadao::scrape(factory.standard_client().ok(), query),
        )));
        tasks.push(Box::pin(run_cached_scraper(
            SourceName::TrangTrang,
            query,
            query_type,
            cache,
            trangtrang::scrape(factory.standard_client().ok(), query),
        )));
    }

    let google = run_cached_scraper(
        SourceName::Google,
        query,
        query_type,
        cache,
        google::scrape(factory.clone(), query, query_type, proxy_pool),
    );

    let mut results = futures::future::join_all(tasks).await;
    let google = google.await;
    if google.success {
        results.push(google);
    } else {
        results.push(google);
        let ddg = run_cached_scraper(
            SourceName::DuckDuckGo,
            query,
            query_type,
            cache,
            duckduckgo::scrape(base_client, query, query_type),
        )
        .await;
        results.push(ddg);
    }

    if results.iter().all(|item| !item.success) {
        results.push(ScrapedResult {
            source: SourceName::DuckDuckGo,
            query: query.to_owned(),
            success: false,
            reports: Vec::new(),
            search_results: Vec::new(),
            metadata: json!({ "fallback": "all_failed" }),
            raw_html: None,
            duration_ms: 0,
            error: Some("all scrapers failed".to_owned()),
        });
    }

    results
}

async fn run_cached_scraper<F>(
    source: SourceName,
    query: &str,
    query_type: QueryType,
    cache: Option<&CacheService>,
    future: F,
) -> ScrapedResult
where
    F: std::future::Future<Output = ScrapedResult>,
{
    let cache_key = format!("{source:?}");
    if let Some(cache) = cache {
        if let Some(cached) = cache
            .get_scrape(query_type, query, &cache_key)
            .await
            .ok()
            .flatten()
        {
            return cached;
        }
    }

    let result = run_with_timeout(source, query, future).await;
    if result.success {
        if let Some(cache) = cache {
            if let Err(error) = cache
                .set_scrape(query_type, query, &cache_key, &result, 24)
                .await
            {
                tracing::warn!("failed to cache scrape result for {cache_key}: {error}");
            }
        }
    }

    result
}

async fn run_with_timeout<F>(source: SourceName, query: &str, future: F) -> ScrapedResult
where
    F: std::future::Future<Output = ScrapedResult>,
{
    let started_at = Instant::now();
    match tokio::time::timeout(SCRAPER_TIMEOUT, future).await {
        Ok(result) => result,
        Err(_) => ScrapedResult::failure(source, query, started_at, "scraper timed out"),
    }
}
