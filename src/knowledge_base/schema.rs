use crate::error::AppResult;

use super::KnowledgeBase;

impl KnowledgeBase {
    pub async fn ensure_schema(&self) -> AppResult<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS subjects (
                id UUID PRIMARY KEY,
                value TEXT NOT NULL,
                subject_type TEXT NOT NULL,
                risk_score REAL NOT NULL DEFAULT 0,
                risk_level TEXT NOT NULL DEFAULT 'unknown',
                report_count INTEGER NOT NULL DEFAULT 0,
                investigation_count INTEGER NOT NULL DEFAULT 0,
                first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE(value, subject_type)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS investigations (
                id UUID PRIMARY KEY,
                subject_id UUID NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                query TEXT NOT NULL,
                query_type TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                risk_score_numeric REAL NOT NULL,
                confidence REAL NOT NULL,
                sources_analyzed INTEGER NOT NULL,
                sources_success_ratio REAL NOT NULL,
                quality_score REAL NOT NULL,
                duration_ms BIGINT NOT NULL,
                detective_report TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS evidence (
                id UUID PRIMARY KEY,
                subject_id UUID NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                investigation_id UUID REFERENCES investigations(id) ON DELETE SET NULL,
                source TEXT NOT NULL,
                evidence_type TEXT NOT NULL,
                data JSONB NOT NULL,
                risk_signals TEXT[] NOT NULL DEFAULT '{}',
                mentioned_subjects TEXT[] NOT NULL DEFAULT '{}',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS subject_links (
                id UUID PRIMARY KEY,
                subject_a_id UUID NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                subject_b_id UUID NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                link_type TEXT NOT NULL,
                strength REAL NOT NULL DEFAULT 1,
                evidence_ids UUID[] NOT NULL DEFAULT '{}',
                first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                CONSTRAINT chk_subject_order CHECK (subject_a_id < subject_b_id),
                UNIQUE(subject_a_id, subject_b_id, link_type)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS user_reports (
                id UUID PRIMARY KEY,
                subject_id UUID NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                reporter_ip_hash TEXT NOT NULL,
                description TEXT NOT NULL,
                category TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                reviewed_by TEXT,
                reviewed_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS media (
                id UUID PRIMARY KEY,
                entity_type TEXT NOT NULL,
                entity_id UUID NOT NULL,
                file_path TEXT NOT NULL,
                original_url TEXT,
                content_type TEXT,
                file_size_bytes BIGINT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS crawl_failures (
                id UUID PRIMARY KEY,
                url TEXT NOT NULL,
                error_phase TEXT NOT NULL,
                error_message TEXT,
                retry_count INT NOT NULL DEFAULT 0,
                resolved BOOLEAN NOT NULL DEFAULT FALSE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        for statement in [
            "CREATE INDEX IF NOT EXISTS idx_subject_lookup ON subjects(value, subject_type)",
            "CREATE INDEX IF NOT EXISTS idx_subject_risk ON subjects(risk_level, risk_score DESC)",
            "CREATE INDEX IF NOT EXISTS idx_investigations_subject ON investigations(subject_id, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_evidence_subject ON evidence(subject_id, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_evidence_data_gin ON evidence USING GIN(data)",
            "CREATE INDEX IF NOT EXISTS idx_evidence_mentions_gin ON evidence USING GIN(mentioned_subjects)",
            "CREATE INDEX IF NOT EXISTS idx_links_a ON subject_links(subject_a_id)",
            "CREATE INDEX IF NOT EXISTS idx_links_b ON subject_links(subject_b_id)",
            "CREATE INDEX IF NOT EXISTS idx_reports_status ON user_reports(status, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_reports_subject ON user_reports(subject_id, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_reports_ip_hash ON user_reports(reporter_ip_hash, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_media_entity ON media(entity_type, entity_id)",
            "CREATE INDEX IF NOT EXISTS idx_media_original_url ON media(original_url) WHERE original_url IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS idx_crawl_failures_unresolved ON crawl_failures(resolved, retry_count) WHERE resolved = false",
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }

        if let Err(error) = sqlx::query(
            "CREATE MATERIALIZED VIEW IF NOT EXISTS subject_risk_overview AS
             SELECT
                s.id,
                s.value,
                s.subject_type,
                s.risk_score,
                s.risk_level,
                s.report_count,
                s.investigation_count,
                MAX(i.created_at) AS last_investigation_at
             FROM subjects s
             LEFT JOIN investigations i ON i.subject_id = s.id
             GROUP BY s.id",
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!("knowledge base materialized view disabled: {error}");
            return Ok(());
        }
        if let Err(error) = sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_subject_risk_overview_id
             ON subject_risk_overview(id)",
        )
        .execute(&self.pool)
        .await
        {
            tracing::warn!("knowledge base materialized view index disabled: {error}");
        }

        Ok(())
    }

    pub async fn refresh_materialized_view(&self) -> AppResult<()> {
        sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY subject_risk_overview")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
