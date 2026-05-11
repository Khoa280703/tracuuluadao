mod agents;
mod api;
mod cache;
mod config;
mod error;
mod pipeline;
mod scrapers;

use std::sync::Arc;

use crate::api::router;
use crate::config::AppConfig;
use crate::error::AppResult;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(AppConfig::from_env()?);
    let state = api::build_state(config.clone()).await?;
    let app = router(state);

    let listener =
        tokio::net::TcpListener::bind((config.app_host.as_str(), config.app_port)).await?;
    tracing::info!("listening on {}:{}", config.app_host, config.app_port);
    axum::serve(listener, app).await?;
    Ok(())
}
