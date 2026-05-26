use std::net::SocketAddr;
use std::sync::Arc;

use tracuuluadao::api::{self, router};
use tracuuluadao::config::AppConfig;
use tracuuluadao::error::AppResult;
use tracuuluadao::logging;

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();
    let config = Arc::new(AppConfig::from_env()?);
    let _log_guards = logging::init()?;
    let state = api::build_state(config.clone()).await?;
    let app = router(state);

    let listener =
        tokio::net::TcpListener::bind((config.app_host.as_str(), config.app_port)).await?;
    tracing::info!("listening on {}:{}", config.app_host, config.app_port);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
