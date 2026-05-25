use std::env;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_host: String,
    pub app_port: u16,
    pub database_url: Option<String>,
    pub redis_url: Option<String>,
    pub admin_api_key: Option<String>,
    pub investigation_report_ttl_secs: u64,
    pub proxy_dir: String,
    pub agent_config_dir: String,
}

impl AppConfig {
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            app_host: env_var("APP_HOST")?,
            app_port: env_var("APP_PORT")?
                .parse()
                .map_err(|_| AppError::Config("APP_PORT must be a valid u16".to_string()))?,
            database_url: optional_env_var("DATABASE_URL"),
            redis_url: optional_env_var("REDIS_URL"),
            admin_api_key: optional_env_var("ADMIN_API_KEY"),
            investigation_report_ttl_secs: optional_env_var("INVESTIGATION_REPORT_TTL_SECS")
                .map(|value| {
                    value.parse().map_err(|_| {
                        AppError::Config(
                            "INVESTIGATION_REPORT_TTL_SECS must be a valid u64".to_string(),
                        )
                    })
                })
                .transpose()?
                .unwrap_or(3600),
            proxy_dir: env_var("PROXY_DIR")?,
            agent_config_dir: env_var("AGENT_CONFIG_DIR")?,
        })
    }
}

fn env_var(key: &str) -> AppResult<String> {
    env::var(key).map_err(|_| AppError::Config(format!("missing env var: {key}")))
}

fn optional_env_var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}
