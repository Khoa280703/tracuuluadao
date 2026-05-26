---
phase: 1
title: "Media Table Schema + CRUD Service"
status: completed
effort: 1.5h
---

# Phase 1: Media Table Schema + CRUD Service

## Context Links

- [Brainstorm report](../reports/brainstorm-260525-1424-checkscam-bulk-crawler.md)
- [Current schema](../../src/knowledge_base/schema.rs)
- [Current models](../../src/knowledge_base/models.rs)
- [KnowledgeBase mod](../../src/knowledge_base/mod.rs)

## Overview

Add a shared polymorphic `media` table for storing file metadata (images, screenshots, documents) attached to any entity (evidence, user_report, investigation). Create CRUD methods and model.

## Key Insights

- Polymorphic design (entity_type + entity_id) avoids N join tables for each entity
- `file_path` stores relative path from project root — portable across environments
- `original_url` is nullable — only set for crawled/downloaded media, not user uploads (which have no source URL yet, but can be added later)
- No foreign key on entity_id — polymorphic reference intentionally avoids FK constraints to keep schema flexible across entity types

## Requirements

**Functional:**
- Create `media` table with DDL in `ensure_schema()`
- Insert single media record
- Insert batch of media records
- Query media by entity (type + id)
- Delete media by id (with filesystem cleanup left to caller)

**Non-functional:**
- Index on (entity_type, entity_id) for fast lookups
- UUID primary key consistent with other tables

## Architecture

```
media table
├── id (UUID PK)
├── entity_type (TEXT) — 'evidence' | 'user_report' | 'investigation'
├── entity_id (UUID) — polymorphic reference
├── file_path (TEXT) — relative: "evidence/{subject_id}/{hash}.jpg"
├── original_url (TEXT, nullable) — source URL if crawled
├── content_type (TEXT, nullable) — "image/jpeg", "image/png"
├── file_size_bytes (BIGINT, nullable) — file size after download
└── created_at (TIMESTAMPTZ)

Filesystem layout:
data/media/
├── evidence/{subject_id}/{sha256_hash}.{ext}
├── reports/{report_id}/{sha256_hash}.{ext}
└── investigations/{inv_id}/{sha256_hash}.{ext}
```

## Related Code Files

**Modify:**
- `src/knowledge_base/schema.rs` — add media table CREATE + index
- `src/knowledge_base/models.rs` — add `MediaRecord` struct
- `src/knowledge_base/mod.rs` — add `mod media;` and re-export

**Create:**
- `src/knowledge_base/media.rs` — CRUD impl block on KnowledgeBase

## Implementation Steps

### Step 1: Add MediaRecord model to `models.rs`

Add at end of file:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MediaRecord {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub file_path: String,
    pub original_url: Option<String>,
    pub content_type: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub created_at: DateTime<Utc>,
}
```

### Step 2: Add media table DDL to `schema.rs`

Insert after the `user_reports` CREATE TABLE block (before the index loop):

```rust
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
```

Add to the index array:

```rust
"CREATE INDEX IF NOT EXISTS idx_media_entity ON media(entity_type, entity_id)",
"CREATE INDEX IF NOT EXISTS idx_media_original_url ON media(original_url) WHERE original_url IS NOT NULL",
```

The `idx_media_original_url` partial index enables fast dedup checks during crawl (skip already-downloaded images).

### Step 3: Create `media.rs` with CRUD methods

```rust
// src/knowledge_base/media.rs

use uuid::Uuid;

use crate::error::AppResult;

use super::KnowledgeBase;
use super::models::MediaRecord;

impl KnowledgeBase {
    /// Insert a single media record. Returns the inserted record's ID.
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

    /// Insert a batch of media records. Each tuple:
    /// (entity_type, entity_id, file_path, original_url, content_type, file_size_bytes)
    pub async fn insert_media_batch(
        &self,
        items: &[(String, Uuid, String, Option<String>, Option<String>, Option<i64>)],
    ) -> AppResult<Vec<Uuid>> {
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            let id = self
                .insert_media(
                    &item.0,
                    item.1,
                    &item.2,
                    item.3.as_deref(),
                    item.4.as_deref(),
                    item.5,
                )
                .await?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// Check if media with this original_url already exists (dedup for crawl).
    pub async fn media_exists_by_url(&self, original_url: &str) -> AppResult<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM media WHERE original_url = $1)",
        )
        .bind(original_url)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    /// Get all media for an entity.
    pub async fn get_media_for_entity(
        &self,
        entity_type: &str,
        entity_id: Uuid,
    ) -> AppResult<Vec<MediaRecord>> {
        sqlx::query_as::<_, MediaRecord>(
            "SELECT id, entity_type, entity_id, file_path,
                    original_url, content_type, file_size_bytes, created_at
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
}
```

### Step 4: Register module in `mod.rs`

Add `mod media;` to the module list in `src/knowledge_base/mod.rs`. Add `MediaRecord` to the re-exports.

### Step 5: Compile check

```bash
cargo check
```

## Todo List

- [x] Add `MediaRecord` struct to `models.rs`
- [x] Add media table DDL + indexes to `schema.rs` `ensure_schema()`
- [x] Create `src/knowledge_base/media.rs` with CRUD methods
- [x] Register `mod media` in `mod.rs`, add re-export
- [x] `cargo check` — verify compilation

## Success Criteria

- [x] `cargo check` passes with no errors
- [x] `ensure_schema()` creates `media` table + indexes on fresh DB
- [x] `insert_media`, `insert_media_batch`, `media_exists_by_url`, `get_media_for_entity` compile and have correct SQL

## Risk Assessment

- **Low risk:** Pure additive schema change. No existing tables modified.
- **Polymorphic FK:** No FK on entity_id intentional — avoids complexity. Application logic ensures referential integrity.
- **Index bloat:** Two indexes is reasonable. Partial index on original_url only indexes non-null rows.

## Next Steps

Phase 2 depends on this — media CRUD is used by the crawler to store downloaded images.
