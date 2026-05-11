use axum::{Json, extract::State};
use serde::Serialize;

use super::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    ok: bool,
    proxies_loaded: bool,
    uptime_seconds: u64,
    cache_enabled: bool,
}

pub async fn handler(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        proxies_loaded: !state.proxy_pool.is_empty(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        cache_enabled: state.cache.is_some(),
    })
}
