pub mod health;
pub mod investigate;

use std::sync::Arc;
use std::time::Instant;

use axum::{Router, routing::get};
use tower_http::cors::CorsLayer;

use crate::agents::config::{AgentRegistry, EndpointOverrides};
use crate::agents::hot_reload::start_hot_reload;
use crate::agents::llm_client::LlmClient;
use crate::cache::{CacheService, start_cleanup_task};
use crate::config::AppConfig;
use crate::error::AppResult;
use crate::scrapers::proxy::ProxyPool;

#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<AgentRegistry>,
    pub llm: Arc<LlmClient>,
    pub proxy_pool: Arc<ProxyPool>,
    pub cache: Option<Arc<CacheService>>,
    pub started_at: Instant,
}

pub async fn build_state(config: Arc<AppConfig>) -> AppResult<AppState> {
    let registry = AgentRegistry::load_all(
        &config.agent_config_dir,
        EndpointOverrides {
            qwen35_endpoint: Some(config.qwen35_endpoint.clone()),
            qwen36_endpoint: Some(config.qwen36_endpoint.clone()),
        },
    )?;
    start_hot_reload(registry.clone())?;
    let llm = Arc::new(LlmClient::new()?);
    let proxy_pool = Arc::new(ProxyPool::load_from_dir(&config.proxy_dir).unwrap_or_default());
    let cache = match config.database_url.as_deref() {
        Some(database_url) => match CacheService::new(database_url) {
            Ok(cache) => {
                let cache = Arc::new(cache);
                if let Err(error) = cache.ensure_schema().await {
                    tracing::warn!("cache disabled: {error}");
                    None
                } else {
                    start_cleanup_task(cache.clone());
                    Some(cache)
                }
            }
            Err(error) => {
                tracing::warn!("cache disabled: {error}");
                None
            }
        },
        None => {
            tracing::warn!("cache disabled: DATABASE_URL missing");
            None
        }
    };

    Ok(AppState {
        registry,
        llm,
        proxy_pool,
        cache,
        started_at: Instant::now(),
    })
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::handler))
        .route("/api/investigate", get(investigate::handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
