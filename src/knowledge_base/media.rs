use uuid::Uuid;

use crate::error::AppResult;

use super::KnowledgeBase;
use super::models::MediaRecord;

impl KnowledgeBase {
    pub async fn insert_media(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        file_path: &str,
        original_url: Option<&str>,
        content_type: Option<&str>,
        file_size_bytes: Option<i64>,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO media (
                id, entity_type, entity_id, file_path,
                original_url, content_type, file_size_bytes
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(entity_type)
        .bind(entity_id)
        .bind(file_path)
        .bind(original_url)
        .bind(content_type)
        .bind(file_size_bytes)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    pub async fn insert_media_batch(
        &self,
        items: &[(
            String,
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
        )],
    ) -> AppResult<Vec<Uuid>> {
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            ids.push(
                self.insert_media(
                    &item.0,
                    item.1,
                    &item.2,
                    item.3.as_deref(),
                    item.4.as_deref(),
                    item.5,
                )
                .await?,
            );
        }
        Ok(ids)
    }

    pub async fn media_exists_by_url(&self, original_url: &str) -> AppResult<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM media WHERE original_url = $1)",
        )
        .bind(original_url)
        .fetch_one(&self.pool)
        .await?;

        Ok(exists)
    }

    pub async fn media_exists_for_entity_url(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        original_url: &str,
    ) -> AppResult<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                SELECT 1
                FROM media
                WHERE entity_type = $1
                  AND entity_id = $2
                  AND original_url = $3
            )",
        )
        .bind(entity_type)
        .bind(entity_id)
        .bind(original_url)
        .fetch_one(&self.pool)
        .await?;

        Ok(exists)
    }

    pub async fn get_media_by_original_url(
        &self,
        original_url: &str,
    ) -> AppResult<Option<MediaRecord>> {
        sqlx::query_as::<_, MediaRecord>(
            "SELECT
                id, entity_type, entity_id, file_path, original_url,
                content_type, file_size_bytes, created_at
             FROM media
             WHERE original_url = $1
             ORDER BY created_at ASC
             LIMIT 1",
        )
        .bind(original_url)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_media_for_entity(
        &self,
        entity_type: &str,
        entity_id: Uuid,
    ) -> AppResult<Vec<MediaRecord>> {
        sqlx::query_as::<_, MediaRecord>(
            "SELECT
                id, entity_type, entity_id, file_path, original_url,
                content_type, file_size_bytes, created_at
             FROM media
             WHERE entity_type = $1 AND entity_id = $2
             ORDER BY created_at",
        )
        .bind(entity_type)
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn delete_media(&self, id: Uuid) -> AppResult<bool> {
        let deleted = sqlx::query("DELETE FROM media WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();

        Ok(deleted > 0)
    }
}
