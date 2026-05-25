pub mod admin;
pub mod health;
pub mod investigate;
pub mod reports;
pub mod subjects;

use std::sync::Arc;
use std::time::Instant;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::CorsLayer;

use crate::agents::config::AgentRegistry;
use crate::agents::hot_reload::start_hot_reload;
use crate::agents::llm_client::LlmClient;
use crate::cache::{CacheService, start_cleanup_task};
use crate::config::AppConfig;
use crate::error::AppResult;
use crate::knowledge_base::{KnowledgeBase, start_refresh_task};
use crate::report_store::InvestigationReportStore;
use crate::scrapers::proxy::ProxyPool;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub registry: Arc<AgentRegistry>,
    pub llm: Arc<LlmClient>,
    pub proxy_pool: Arc<ProxyPool>,
    pub cache: Option<Arc<CacheService>>,
    pub knowledge_base: Option<Arc<KnowledgeBase>>,
    pub report_store: Option<Arc<InvestigationReportStore>>,
    pub started_at: Instant,
}

pub async fn build_state(config: Arc<AppConfig>) -> AppResult<AppState> {
    let registry = AgentRegistry::load_all(&config.agent_config_dir)?;
    start_hot_reload(registry.clone())?;
    let llm = Arc::new(LlmClient::new()?);
    let proxy_pool = Arc::new(ProxyPool::load_from_dir(&config.proxy_dir).unwrap_or_default());
    let (cache, knowledge_base) = match config.database_url.as_deref() {
        Some(database_url) => match CacheService::connect_pool(database_url) {
            Ok(pool) => {
                let cache = Arc::new(CacheService::from_pool(pool.clone()));
                let knowledge_base = Arc::new(KnowledgeBase::new(pool));
                let cache_available = if let Err(error) = cache.ensure_schema().await {
                    tracing::warn!("cache disabled: {error}");
                    None
                } else {
                    start_cleanup_task(cache.clone());
                    Some(cache)
                };
                let knowledge_base_available =
                    if let Err(error) = knowledge_base.ensure_schema().await {
                        tracing::warn!("knowledge base disabled: {error}");
                        None
                    } else {
                        start_refresh_task(knowledge_base.clone());
                        Some(knowledge_base)
                    };
                (cache_available, knowledge_base_available)
            }
            Err(error) => {
                tracing::warn!("database services disabled: {error}");
                (None, None)
            }
        },
        None => {
            tracing::warn!("database services disabled: DATABASE_URL missing");
            (None, None)
        }
    };
    let report_store = match config.redis_url.as_deref() {
        Some(redis_url) => {
            match InvestigationReportStore::new(redis_url, config.investigation_report_ttl_secs) {
                Ok(store) => Some(Arc::new(store)),
                Err(error) => {
                    tracing::warn!("report store disabled: {error}");
                    None
                }
            }
        }
        None => {
            tracing::warn!("report store disabled: REDIS_URL missing");
            None
        }
    };

    Ok(AppState {
        config,
        registry,
        llm,
        proxy_pool,
        cache,
        knowledge_base,
        report_store,
        started_at: Instant::now(),
    })
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::handler))
        .route("/api/investigate", get(investigate::handler))
        .route("/api/investigate/report", get(investigate::report_handler))
        .route("/api/subjects/{value}", get(subjects::get_subject))
        .route("/api/subjects/{value}/network", get(subjects::get_network))
        .route("/api/reports", post(reports::submit_report))
        .route("/api/admin/reports", get(admin::list_reports))
        .route(
            "/api/admin/reports/{id}/approve",
            post(admin::approve_report),
        )
        .route("/api/admin/reports/{id}/reject", post(admin::reject_report))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
