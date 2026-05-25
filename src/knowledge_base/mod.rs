mod reports;
mod risk;
mod schema;
mod subjects;

pub mod models;

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::time::MissedTickBehavior;

pub use models::{
    EvidenceInput, HistoricalContextSnapshot, NetworkGraph, SubjectHistory, UserReportWithSubject,
};
pub use risk::{compute_quality_score, risk_level_to_numeric};

#[derive(Debug, Clone)]
pub struct KnowledgeBase {
    pub(crate) pool: PgPool,
}

impl KnowledgeBase {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const MATERIALIZED_VIEW_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub fn start_refresh_task(knowledge_base: Arc<KnowledgeBase>) {
    tokio::spawn(async move {
        if let Err(error) = knowledge_base.refresh_materialized_view().await {
            tracing::warn!("knowledge base view refresh on startup failed: {error}");
        }

        let mut interval = tokio::time::interval(MATERIALIZED_VIEW_REFRESH_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        interval.tick().await;

        loop {
            interval.tick().await;
            if let Err(error) = knowledge_base.refresh_materialized_view().await {
                tracing::warn!("knowledge base view refresh failed: {error}");
            }
        }
    });
}
