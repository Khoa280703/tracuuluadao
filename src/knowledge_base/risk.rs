use sqlx::Row;
use uuid::Uuid;

use crate::error::AppResult;

use super::KnowledgeBase;

pub fn compute_quality_score(
    sources_success_ratio: f32,
    confidence: f32,
    risk_signals_count: usize,
    evidence_urls_count: usize,
) -> f32 {
    let evidence_richness = ((risk_signals_count + evidence_urls_count) as f32 / 10.0).min(1.0);
    ((sources_success_ratio.clamp(0.0, 1.0) * 0.4)
        + (confidence.clamp(0.0, 1.0) * 0.3)
        + (evidence_richness * 0.2)
        + 0.1)
        .clamp(0.0, 1.0)
}

pub fn risk_level_to_numeric(risk_level: &str) -> f32 {
    match risk_level {
        "critical" => 1.0,
        "high" => 0.8,
        "medium" => 0.5,
        "low" => 0.2,
        _ => 0.0,
    }
}

pub fn numeric_to_risk_level(score: f32) -> &'static str {
    match score {
        score if score >= 0.8 => "critical",
        score if score >= 0.6 => "high",
        score if score >= 0.3 => "medium",
        score if score > 0.0 => "low",
        _ => "unknown",
    }
}

impl KnowledgeBase {
    pub async fn recalculate_risk(&self, subject_id: Uuid) -> AppResult<()> {
        let investigation_score = sqlx::query(
            "SELECT COALESCE(
                SUM(risk_score_numeric * quality_score) / NULLIF(SUM(quality_score), 0),
                0
             )::REAL AS score
             FROM investigations
             WHERE subject_id = $1",
        )
        .bind(subject_id)
        .fetch_one(&self.pool)
        .await?
        .try_get::<f32, _>("score")?;

        let approved_report_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_reports WHERE subject_id = $1 AND status = 'approved'",
        )
        .bind(subject_id)
        .fetch_one(&self.pool)
        .await? as i32;

        let report_signal = (approved_report_count as f32 * 0.12).min(0.72);
        let final_score = if investigation_score <= 0.0 {
            report_signal
        } else {
            investigation_score.max((investigation_score * 0.8) + (report_signal * 0.2))
        }
        .clamp(0.0, 1.0);
        let risk_level = numeric_to_risk_level(final_score);

        sqlx::query(
            "UPDATE subjects
             SET risk_score = $2,
                 risk_level = $3,
                 report_count = $4,
                 last_seen_at = NOW()
             WHERE id = $1",
        )
        .bind(subject_id)
        .bind(final_score)
        .bind(risk_level)
        .bind(approved_report_count)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
