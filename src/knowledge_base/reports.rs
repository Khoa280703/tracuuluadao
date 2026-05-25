use uuid::Uuid;

use crate::error::AppResult;

use super::KnowledgeBase;
use super::models::UserReportWithSubject;

impl KnowledgeBase {
    pub async fn check_report_rate_limit(
        &self,
        ip_hash: &str,
        max_per_day: i32,
    ) -> AppResult<bool> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
             FROM user_reports
             WHERE reporter_ip_hash = $1
               AND created_at > NOW() - INTERVAL '24 hours'",
        )
        .bind(ip_hash)
        .fetch_one(&self.pool)
        .await?;

        Ok(count < max_per_day as i64)
    }

    pub async fn check_duplicate_report(&self, ip_hash: &str, subject_id: Uuid) -> AppResult<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                SELECT 1
                FROM user_reports
                WHERE reporter_ip_hash = $1
                  AND subject_id = $2
                  AND created_at > NOW() - INTERVAL '24 hours'
            )",
        )
        .bind(ip_hash)
        .bind(subject_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(!exists)
    }

    pub async fn insert_user_report(
        &self,
        subject_id: Uuid,
        ip_hash: &str,
        description: &str,
        category: Option<&str>,
    ) -> AppResult<Uuid> {
        let report_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO user_reports (
                id, subject_id, reporter_ip_hash, description, category, status
             ) VALUES ($1, $2, $3, $4, $5, 'pending')",
        )
        .bind(report_id)
        .bind(subject_id)
        .bind(ip_hash)
        .bind(description)
        .bind(category)
        .execute(&self.pool)
        .await?;

        Ok(report_id)
    }

    pub async fn list_reports_by_status(
        &self,
        status: &str,
        limit: i32,
        offset: i32,
    ) -> AppResult<Vec<UserReportWithSubject>> {
        sqlx::query_as::<_, UserReportWithSubject>(
            "SELECT
                ur.id,
                ur.subject_id,
                s.value AS subject_value,
                s.subject_type,
                s.risk_level AS subject_risk_level,
                ur.description,
                ur.category,
                ur.status,
                ur.created_at,
                ur.reviewed_by,
                ur.reviewed_at
             FROM user_reports ur
             JOIN subjects s ON s.id = ur.subject_id
             WHERE ur.status = $1
             ORDER BY ur.created_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn update_report_status(
        &self,
        report_id: Uuid,
        status: &str,
        reviewed_by: &str,
    ) -> AppResult<Option<Uuid>> {
        sqlx::query_scalar::<_, Uuid>(
            "UPDATE user_reports
             SET status = $2,
                 reviewed_by = $3,
                 reviewed_at = NOW()
             WHERE id = $1
             RETURNING subject_id",
        )
        .bind(report_id)
        .bind(status)
        .bind(reviewed_by)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }
}
