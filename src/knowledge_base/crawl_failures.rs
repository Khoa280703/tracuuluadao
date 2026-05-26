use anyhow::Result;
use uuid::Uuid;

use super::KnowledgeBase;

impl KnowledgeBase {
    pub async fn insert_crawl_failure(
        &self,
        url: &str,
        error_phase: &str,
        error_message: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO crawl_failures (id, url, error_phase, error_message)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(url)
        .bind(error_phase)
        .bind(error_message)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn increment_crawl_failure(
        &self,
        url: &str,
        error_phase: &str,
        error_message: &str,
    ) -> Result<()> {
        let rows = sqlx::query(
            "UPDATE crawl_failures
             SET retry_count = retry_count + 1,
                 error_message = $3,
                 error_phase = $2,
                 updated_at = NOW()
             WHERE url = $1 AND resolved = false",
        )
        .bind(url)
        .bind(error_phase)
        .bind(error_message)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows == 0 {
            self.insert_crawl_failure(url, error_phase, error_message)
                .await?;
        }
        Ok(())
    }

    pub async fn mark_failure_resolved(&self, url: &str) -> Result<()> {
        sqlx::query("UPDATE crawl_failures SET resolved = true, updated_at = NOW() WHERE url = $1")
            .bind(url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_unresolved_failure_urls(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT url FROM crawl_failures
             WHERE resolved = false
             ORDER BY retry_count ASC, created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(url,)| url).collect())
    }
}
