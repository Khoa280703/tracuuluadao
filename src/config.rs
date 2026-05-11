use std::env;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_host: String,
    pub app_port: u16,
    pub database_url: Option<String>,
    pub proxy_dir: String,
    pub agent_config_dir: String,
    pub qwen35_endpoint: String,
    pub qwen36_endpoint: String,
}

impl AppConfig {
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            app_host: env_var("APP_HOST")?,
            app_port: env_var("APP_PORT")?
                .parse()
                .map_err(|_| AppError::Config("APP_PORT must be a valid u16".to_string()))?,
            database_url: optional_env_var("DATABASE_URL"),
            proxy_dir: env_var("PROXY_DIR")?,
            agent_config_dir: env_var("AGENT_CONFIG_DIR")?,
            qwen35_endpoint: env_var("QWEN35_ENDPOINT")?,
            qwen36_endpoint: env_var("QWEN36_ENDPOINT")?,
        })
    }
}

fn env_var(key: &str) -> AppResult<String> {
    env::var(key).map_err(|_| AppError::Config(format!("missing env var: {key}")))
}

fn optional_env_var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}
