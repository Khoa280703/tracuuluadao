use std::env;
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

use crate::error::{AppError, AppResult};

const DEFAULT_LOG_DIR: &str = "logs";
const DEFAULT_LOG_PREFIX: &str = "backend.log";
const DEFAULT_LOG_FILTER: &str = "info,tracuuluadao=debug,tower_http=info,axum=info";

pub fn init() -> AppResult<Vec<WorkerGuard>> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(DEFAULT_LOG_FILTER))
        .map_err(|error| AppError::Server(format!("failed to initialize log filter: {error}")))?;
    let log_dir = env::var("APP_LOG_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LOG_DIR.to_string());
    let log_prefix = env::var("APP_LOG_FILE_PREFIX")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LOG_PREFIX.to_string());

    fs::create_dir_all(&log_dir).map_err(|error| {
        AppError::Server(format!("failed to create log dir {}: {error}", log_dir))
    })?;

    let stdout_layer = Layer::new()
        .with_ansi(io::stdout().is_terminal())
        .with_target(true)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    let file_path = PathBuf::from(&log_dir);
    let file_appender = tracing_appender::rolling::daily(file_path, log_prefix);
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = Layer::new()
        .with_ansi(false)
        .with_target(true)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_writer(file_writer);

    Registry::default()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(vec![file_guard])
}
