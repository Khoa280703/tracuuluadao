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
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::pipeline::QueryType;
use crate::scrapers::http_client::HttpClientFactory;
use crate::scrapers::proxy::ProxyPool;

const DEFAULT_SCRAPER_TIMEOUT: Duration = Duration::from_secs(30);
const CHECKSCAM_SCRAPER_TIMEOUT: Duration = Duration::from_secs(90);
const GOOGLE_SCRAPER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SourceName {
    CheckScam,
    ChongLuaDao,
    TrangTrang,
    TinNhiemMang,
    Google,
    DuckDuckGo,
}

impl SourceName {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::CheckScam => "CheckScam.vn",
            Self::ChongLuaDao => "ChongLuaDao.vn",
            Self::TrangTrang => "TrangTrang",
            Self::TinNhiemMang => "TinNhiemMang.vn",
            Self::Google => "Google",
            Self::DuckDuckGo => "DuckDuckGo",
        }
    }

    pub fn is_search_engine(&self) -> bool {
        matches!(self, Self::Google | Self::DuckDuckGo)
    }
}

pub enum ScraperProgress {
    Starting(SourceName),
    Completed(ScrapedResult),
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
    progress_tx: Option<mpsc::UnboundedSender<ScraperProgress>>,
) -> Vec<ScrapedResult> {
    tracing::info!(
        query = %query,
        query_type = query_type.as_str(),
        proxy_pool_loaded = proxy_pool.is_some_and(|pool| !pool.is_empty()),
        "starting scraper fanout"
    );
    let factory = HttpClientFactory::default();
    let base_client = factory.standard_client().ok();
    let mut tasks = FuturesUnordered::<BoxFuture<'_, ScrapedResult>>::new();

    let emit_starting = |source: SourceName| {
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(ScraperProgress::Starting(source));
        }
    };

    if matches!(
        query_type,
        QueryType::Phone | QueryType::Bank | QueryType::Url
    ) {
        emit_starting(SourceName::CheckScam);
        tasks.push(Box::pin(run_with_timeout(
            SourceName::CheckScam,
            query,
            checkscam::scrape(factory.clone(), proxy_pool, query),
        )));
        emit_starting(SourceName::TinNhiemMang);
        tasks.push(Box::pin(run_with_timeout(
            SourceName::TinNhiemMang,
            query,
            tinnhiemmang::scrape(factory.standard_client().ok(), query),
        )));
    }

    if matches!(query_type, QueryType::Phone) {
        emit_starting(SourceName::ChongLuaDao);
        tasks.push(Box::pin(run_with_timeout(
            SourceName::ChongLuaDao,
            query,
            chongluadao::scrape(factory.standard_client().ok(), query),
        )));
        emit_starting(SourceName::TrangTrang);
        tasks.push(Box::pin(run_with_timeout(
            SourceName::TrangTrang,
            query,
            trangtrang::scrape(factory.standard_client().ok(), query),
        )));
    }

    emit_starting(SourceName::Google);
    let google = run_with_timeout(
        SourceName::Google,
        query,
        google::scrape(factory.clone(), query, query_type, proxy_pool),
    );

    let mut results = Vec::new();
    let progress_tx = progress_tx;
    while let Some(result) = futures::StreamExt::next(&mut tasks).await {
        log_scraper_result(&result);
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(ScraperProgress::Completed(result.clone()));
        }
        results.push(result);
    }

    let google = google.await;
    log_scraper_result(&google);
    if let Some(tx) = progress_tx.as_ref() {
        let _ = tx.send(ScraperProgress::Completed(google.clone()));
    }
    if google.success {
        results.push(google);
    } else {
        results.push(google);
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(ScraperProgress::Starting(SourceName::DuckDuckGo));
        }
        let ddg = run_with_timeout(
            SourceName::DuckDuckGo,
            query,
            duckduckgo::scrape(base_client, query, query_type),
        )
        .await;
        log_scraper_result(&ddg);
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(ScraperProgress::Completed(ddg.clone()));
        }
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

async fn run_with_timeout<F>(source: SourceName, query: &str, future: F) -> ScrapedResult
where
    F: std::future::Future<Output = ScrapedResult>,
{
    let started_at = Instant::now();
    let timeout = scraper_timeout(source);
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => {
            let duration_ms = started_at.elapsed().as_millis() as u64;
            tracing::warn!(
                source = ?source,
                query = %query,
                duration_ms,
                timeout_ms = timeout.as_millis() as u64,
                "scraper timed out"
            );
            ScrapedResult::failure(source, query, started_at, "scraper timed out")
        }
    }
}

fn scraper_timeout(source: SourceName) -> Duration {
    match source {
        SourceName::CheckScam => CHECKSCAM_SCRAPER_TIMEOUT,
        SourceName::Google => GOOGLE_SCRAPER_TIMEOUT,
        _ => DEFAULT_SCRAPER_TIMEOUT,
    }
}

fn log_scraper_result(result: &ScrapedResult) {
    if result.success {
        tracing::info!(
            source = ?result.source,
            query = %result.query,
            reports = result.reports.len(),
            search_results = result.search_results.len(),
            duration_ms = result.duration_ms,
            metadata = %result.metadata,
            "scraper completed"
        );
    } else {
        tracing::warn!(
            source = ?result.source,
            query = %result.query,
            duration_ms = result.duration_ms,
            error = %result.error.as_deref().unwrap_or("unknown error"),
            metadata = %result.metadata,
            "scraper failed"
        );
    }
}
