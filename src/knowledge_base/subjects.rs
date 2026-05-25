use sqlx::query_as;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

use super::KnowledgeBase;
use super::models::{
    EvidenceInput, HistoricalContextSnapshot, InvestigationRecord, LinkedSubject, NetworkEdge,
    NetworkGraph, NetworkNode, Subject, SubjectHistory, UserReportRecord,
};

impl KnowledgeBase {
    pub async fn upsert_subject(&self, value: &str, subject_type: &str) -> AppResult<Uuid> {
        let normalized_value = normalize_subject_value(value, subject_type);
        if normalized_value.is_empty() {
            return Err(AppError::Config(
                "subject value is empty after normalization".to_string(),
            ));
        }
        let subject = query_as::<_, Subject>(
            "INSERT INTO subjects (
                id, value, subject_type, first_seen_at, last_seen_at
             ) VALUES ($1, $2, $3, NOW(), NOW())
             ON CONFLICT(value, subject_type) DO UPDATE
             SET last_seen_at = NOW()
             RETURNING
                id, value, subject_type, risk_score, risk_level, report_count,
                investigation_count, first_seen_at, last_seen_at",
        )
        .bind(Uuid::new_v4())
        .bind(normalized_value)
        .bind(subject_type)
        .fetch_one(&self.pool)
        .await?;

        Ok(subject.id)
    }

    pub async fn insert_investigation(
        &self,
        subject_id: Uuid,
        query: &str,
        query_type: &str,
        risk_level: &str,
        risk_score_numeric: f32,
        confidence: f32,
        sources_analyzed: i32,
        sources_success_ratio: f32,
        quality_score: f32,
        duration_ms: i64,
        detective_report: Option<&str>,
    ) -> AppResult<Uuid> {
        let investigation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO investigations (
                id, subject_id, query, query_type, risk_level, risk_score_numeric,
                confidence, sources_analyzed, sources_success_ratio, quality_score,
                duration_ms, detective_report
             ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10,
                $11, $12
             )",
        )
        .bind(investigation_id)
        .bind(subject_id)
        .bind(query)
        .bind(query_type)
        .bind(risk_level)
        .bind(risk_score_numeric)
        .bind(confidence)
        .bind(sources_analyzed)
        .bind(sources_success_ratio)
        .bind(quality_score)
        .bind(duration_ms)
        .bind(detective_report)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "UPDATE subjects
             SET investigation_count = investigation_count + 1,
                 last_seen_at = NOW()
             WHERE id = $1",
        )
        .bind(subject_id)
        .execute(&self.pool)
        .await?;

        Ok(investigation_id)
    }

    pub async fn insert_evidence_batch(&self, items: Vec<EvidenceInput>) -> AppResult<()> {
        for item in items {
            sqlx::query(
                "INSERT INTO evidence (
                    id, subject_id, investigation_id, source, evidence_type,
                    data, risk_signals, mentioned_subjects
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(Uuid::new_v4())
            .bind(item.subject_id)
            .bind(item.investigation_id)
            .bind(item.source)
            .bind(item.evidence_type)
            .bind(item.data)
            .bind(item.risk_signals)
            .bind(item.mentioned_subjects)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn upsert_subject_link(
        &self,
        subject_a_id: Uuid,
        subject_b_id: Uuid,
        link_type: &str,
        evidence_id: Option<Uuid>,
    ) -> AppResult<()> {
        let (subject_a_id, subject_b_id) = if subject_a_id < subject_b_id {
            (subject_a_id, subject_b_id)
        } else {
            (subject_b_id, subject_a_id)
        };
        let evidence_ids = evidence_id.map(|id| vec![id]).unwrap_or_default();

        sqlx::query(
            "INSERT INTO subject_links (
                id, subject_a_id, subject_b_id, link_type, strength, evidence_ids
             ) VALUES ($1, $2, $3, $4, 1, $5)
             ON CONFLICT(subject_a_id, subject_b_id, link_type) DO UPDATE
             SET strength = subject_links.strength + 1,
                 evidence_ids = (
                    SELECT ARRAY(
                        SELECT DISTINCT unnest(subject_links.evidence_ids || EXCLUDED.evidence_ids)
                    )
                 ),
                 last_seen_at = NOW()",
        )
        .bind(Uuid::new_v4())
        .bind(subject_a_id)
        .bind(subject_b_id)
        .bind(link_type)
        .bind(evidence_ids)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_subject_by_value(
        &self,
        value: &str,
        subject_type: &str,
    ) -> AppResult<Option<Subject>> {
        let normalized_value = normalize_subject_value(value, subject_type);
        if normalized_value.is_empty() {
            return Ok(None);
        }
        query_as::<_, Subject>(
            "SELECT
                id, value, subject_type, risk_score, risk_level, report_count,
                investigation_count, first_seen_at, last_seen_at
             FROM subjects
             WHERE value = $1 AND subject_type = $2",
        )
        .bind(normalized_value)
        .bind(subject_type)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_subject_history(
        &self,
        subject_id: Uuid,
        limit: i64,
    ) -> AppResult<SubjectHistory> {
        let subject = query_as::<_, Subject>(
            "SELECT
                id, value, subject_type, risk_score, risk_level, report_count,
                investigation_count, first_seen_at, last_seen_at
             FROM subjects
             WHERE id = $1",
        )
        .bind(subject_id)
        .fetch_one(&self.pool)
        .await?;
        let recent_investigations = query_as::<_, InvestigationRecord>(
            "SELECT
                id, risk_level, risk_score_numeric, confidence,
                sources_analyzed, sources_success_ratio, quality_score, created_at
             FROM investigations
             WHERE subject_id = $1
             ORDER BY created_at DESC
             LIMIT $2",
        )
        .bind(subject_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let approved_reports = query_as::<_, UserReportRecord>(
            "SELECT id, description, category, status, created_at
             FROM user_reports
             WHERE subject_id = $1 AND status = 'approved'
             ORDER BY created_at DESC
             LIMIT $2",
        )
        .bind(subject_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let linked_subjects = query_as::<_, LinkedSubject>(
            "SELECT
                other.id,
                other.value,
                other.subject_type,
                other.risk_score,
                other.risk_level,
                other.report_count,
                other.investigation_count,
                other.first_seen_at,
                other.last_seen_at,
                sl.link_type,
                sl.strength
             FROM subject_links sl
             JOIN subjects other
               ON other.id = CASE
                    WHEN sl.subject_a_id = $1 THEN sl.subject_b_id
                    ELSE sl.subject_a_id
                  END
             WHERE sl.subject_a_id = $1 OR sl.subject_b_id = $1
             ORDER BY sl.strength DESC, other.last_seen_at DESC
             LIMIT $2",
        )
        .bind(subject_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let all_risk_signals = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT signal
             FROM evidence
             CROSS JOIN LATERAL unnest(risk_signals) AS signal
             WHERE subject_id = $1
             LIMIT 50",
        )
        .bind(subject_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(SubjectHistory {
            subject,
            recent_investigations,
            approved_reports,
            linked_subjects,
            all_risk_signals,
        })
    }

    pub async fn get_network(&self, subject_id: Uuid, max_depth: i32) -> AppResult<NetworkGraph> {
        let depth = max_depth.clamp(1, 3);
        let nodes = query_as::<_, NetworkNode>(
            "WITH RECURSIVE network AS (
                SELECT s.id, s.value, s.subject_type, s.risk_level, s.risk_score, 0::INT AS depth
                FROM subjects s
                WHERE s.id = $1
                UNION
                SELECT
                    CASE WHEN sl.subject_a_id = n.id THEN s2.id ELSE s1.id END AS id,
                    CASE WHEN sl.subject_a_id = n.id THEN s2.value ELSE s1.value END AS value,
                    CASE WHEN sl.subject_a_id = n.id THEN s2.subject_type ELSE s1.subject_type END AS subject_type,
                    CASE WHEN sl.subject_a_id = n.id THEN s2.risk_level ELSE s1.risk_level END AS risk_level,
                    CASE WHEN sl.subject_a_id = n.id THEN s2.risk_score ELSE s1.risk_score END AS risk_score,
                    (n.depth + 1)::INT AS depth
                FROM network n
                JOIN subject_links sl ON sl.subject_a_id = n.id OR sl.subject_b_id = n.id
                JOIN subjects s1 ON s1.id = sl.subject_a_id
                JOIN subjects s2 ON s2.id = sl.subject_b_id
                WHERE n.depth < $2
            )
            SELECT DISTINCT ON (id)
                id, value, subject_type, risk_level, risk_score, depth
            FROM network
            ORDER BY id, depth",
        )
        .bind(subject_id)
        .bind(depth)
        .fetch_all(&self.pool)
        .await?;
        let edges = query_as::<_, NetworkEdge>(
            "WITH RECURSIVE network AS (
                SELECT s.id, 0::INT AS depth
                FROM subjects s
                WHERE s.id = $1
                UNION
                SELECT
                    CASE WHEN sl.subject_a_id = n.id THEN sl.subject_b_id ELSE sl.subject_a_id END,
                    (n.depth + 1)::INT AS depth
                FROM network n
                JOIN subject_links sl ON sl.subject_a_id = n.id OR sl.subject_b_id = n.id
                WHERE n.depth < $2
            )
            SELECT DISTINCT
                sl.subject_a_id,
                sl.subject_b_id,
                sl.link_type,
                sl.strength
            FROM subject_links sl
            JOIN (SELECT DISTINCT id FROM network) na ON na.id = sl.subject_a_id
            JOIN (SELECT DISTINCT id FROM network) nb ON nb.id = sl.subject_b_id",
        )
        .bind(subject_id)
        .bind(depth)
        .fetch_all(&self.pool)
        .await?;

        Ok(NetworkGraph {
            root_subject_id: subject_id,
            nodes,
            edges,
        })
    }

    pub fn build_historical_snapshot(history: &SubjectHistory) -> HistoricalContextSnapshot {
        HistoricalContextSnapshot {
            investigation_count: history.subject.investigation_count,
            last_risk_level: history
                .recent_investigations
                .first()
                .map(|item| item.risk_level.clone())
                .unwrap_or_else(|| history.subject.risk_level.clone()),
            approved_reports: history.approved_reports.len() as i32,
            linked_subjects_count: history.linked_subjects.len() as i32,
            first_seen: history.subject.first_seen_at.to_rfc3339(),
            last_seen: history.subject.last_seen_at.to_rfc3339(),
        }
    }
}

fn normalize_subject_value(value: &str, subject_type: &str) -> String {
    let trimmed = value.trim();
    match subject_type {
        "phone" => trimmed
            .chars()
            .filter(|ch| ch.is_ascii_digit())
            .collect::<String>(),
        "bank" => trimmed.split_whitespace().collect::<String>(),
        "url" => trimmed.trim_end_matches('/').to_ascii_lowercase(),
        _ => trimmed.to_string(),
    }
}
