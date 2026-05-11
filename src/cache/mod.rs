use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::time::MissedTickBehavior;

use crate::error::AppResult;
use crate::pipeline::{InvestigationResult, QueryType};
use crate::scrapers::ScrapedResult;

const CACHE_CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone)]
pub struct CacheService {
    pool: PgPool,
}

impl CacheService {
    pub fn new(database_url: &str) -> AppResult<Self> {
        Ok(Self {
            pool: PgPoolOptions::new()
                .max_connections(5)
                .connect_lazy(database_url)?,
        })
    }

    pub async fn ensure_schema(&self) -> AppResult<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS scrape_cache (
                id BIGSERIAL PRIMARY KEY,
                query TEXT NOT NULL,
                query_type TEXT NOT NULL,
                source TEXT NOT NULL,
                result JSONB NOT NULL,
                created_at TIMESTAMPTZ DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_scrape_cache_lookup
             ON scrape_cache(query_type, query, source, expires_at)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS analysis_cache (
                id BIGSERIAL PRIMARY KEY,
                query TEXT NOT NULL,
                agent_name TEXT NOT NULL,
                prompt_hash TEXT NOT NULL,
                input_hash TEXT NOT NULL,
                result JSONB NOT NULL,
                created_at TIMESTAMPTZ DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_analysis_cache_lookup
             ON analysis_cache(query, agent_name, prompt_hash, input_hash, expires_at)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS investigation_cache (
                id BIGSERIAL PRIMARY KEY,
                query TEXT NOT NULL,
                query_type TEXT NOT NULL,
                prompt_hash TEXT NOT NULL,
                risk_level TEXT,
                full_result JSONB NOT NULL,
                created_at TIMESTAMPTZ DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_investigation_lookup
             ON investigation_cache(query, query_type, prompt_hash, expires_at)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn cleanup_expired(&self) -> AppResult<u64> {
        let scrape_deleted = sqlx::query("DELETE FROM scrape_cache WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await?
            .rows_affected();
        let analysis_deleted = sqlx::query("DELETE FROM analysis_cache WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await?
            .rows_affected();
        let investigation_deleted =
            sqlx::query("DELETE FROM investigation_cache WHERE expires_at <= NOW()")
                .execute(&self.pool)
                .await?
                .rows_affected();

        Ok(scrape_deleted + analysis_deleted + investigation_deleted)
    }

    pub async fn get_scrape(
        &self,
        query_type: QueryType,
        query: &str,
        source: &str,
    ) -> AppResult<Option<ScrapedResult>> {
        let row = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT result FROM scrape_cache
             WHERE query_type = $1 AND query = $2 AND source = $3 AND expires_at > NOW()
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(query_type.as_str())
        .bind(query)
        .bind(source)
        .fetch_optional(&self.pool)
        .await?;
        row.map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    pub async fn set_scrape(
        &self,
        query_type: QueryType,
        query: &str,
        source: &str,
        result: &ScrapedResult,
        ttl_hours: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO scrape_cache (query_type, query, source, result, expires_at)
             VALUES ($1, $2, $3, $4, NOW() + make_interval(hours => $5::int))",
        )
        .bind(query_type.as_str())
        .bind(query)
        .bind(source)
        .bind(serde_json::to_value(result)?)
        .bind(ttl_hours)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_analysis(
        &self,
        query: &str,
        agent_name: &str,
        prompt_hash: &str,
        input_hash: &str,
    ) -> AppResult<Option<serde_json::Value>> {
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT result FROM analysis_cache
             WHERE query = $1 AND agent_name = $2 AND prompt_hash = $3 AND input_hash = $4 AND expires_at > NOW()
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(query)
        .bind(agent_name)
        .bind(prompt_hash)
        .bind(input_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn set_analysis(
        &self,
        query: &str,
        agent_name: &str,
        prompt_hash: &str,
        input_hash: &str,
        result: &serde_json::Value,
        ttl_hours: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO analysis_cache (query, agent_name, prompt_hash, input_hash, result, expires_at)
             VALUES ($1, $2, $3, $4, $5, NOW() + make_interval(hours => $6::int))",
        )
        .bind(query)
        .bind(agent_name)
        .bind(prompt_hash)
        .bind(input_hash)
        .bind(result)
        .bind(ttl_hours)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_full_investigation(
        &self,
        query_type: QueryType,
        query: &str,
        prompt_hash: &str,
    ) -> AppResult<Option<InvestigationResult>> {
        let row = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT full_result FROM investigation_cache
             WHERE query_type = $1 AND query = $2 AND prompt_hash = $3 AND expires_at > NOW()
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(query_type.as_str())
        .bind(query)
        .bind(prompt_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    pub async fn set_full_investigation(
        &self,
        query_type: QueryType,
        query: &str,
        prompt_hash: &str,
        result: &InvestigationResult,
        ttl_hours: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO investigation_cache (query_type, query, prompt_hash, risk_level, full_result, expires_at)
             VALUES ($1, $2, $3, $4, $5, NOW() + make_interval(hours => $6::int))",
        )
        .bind(query_type.as_str())
        .bind(query)
        .bind(prompt_hash)
        .bind(&result.risk_level)
        .bind(serde_json::to_value(result)?)
        .bind(ttl_hours)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

pub fn start_cleanup_task(cache: Arc<CacheService>) {
    tokio::spawn(async move {
        if let Err(error) = cache.cleanup_expired().await {
            tracing::warn!("cache cleanup on startup failed: {error}");
        }

        let mut interval = tokio::time::interval(CACHE_CLEANUP_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        interval.tick().await;

        loop {
            interval.tick().await;
            match cache.cleanup_expired().await {
                Ok(deleted) if deleted > 0 => {
                    tracing::info!("cache cleanup removed {deleted} expired rows");
                }
                Ok(_) => {}
                Err(error) => tracing::warn!("cache cleanup failed: {error}"),
            }
        }
    });
}
