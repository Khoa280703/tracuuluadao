# Phase 1: Schema & KnowledgeBase Service

## Context
- [Brainstorm report](../../reports/brainstorm-260523-2259-persistent-knowledge-base.md)
- Current cache: `src/cache/mod.rs` — ephemeral, TTL 1h, giữ nguyên
- DB pool pattern: `CacheService::new()` dùng `connect_lazy` + `ensure_schema()`

## Overview
- **Priority:** P1
- **Status:** Complete
- **Progress:** 100%
- **Effort:** 6h
- Create 5 PostgreSQL tables + 1 materialized view. Implement `KnowledgeBase` service in Rust with CRUD + ingest + query + risk recalculation.

## Key Insights
- Reuse existing `PgPool` from `CacheService` — share connection pool, không tạo pool mới
- `ensure_schema()` pattern đã có sẵn → follow same pattern cho KB tables
- Evidence dùng JSONB → serialize `AgentSummary`, `AgentExtraction`, `ScrapedResult` trực tiếp
- `subject_links` dùng CHECK constraint `subject_a_id < subject_b_id` → undirected graph

## Requirements

### Functional
- UPSERT subjects (phone/bank/url) with risk aggregation
- INSERT investigations with detective report
- INSERT evidence with flexible JSONB data
- UPSERT subject_links with strength increment
- INSERT user_reports with status tracking
- Query subject history (investigations, evidence, links, reports)
- Recalculate subject risk_score from evidence + investigations
- Materialized view refresh (periodic)

### Non-functional
- Share PgPool with CacheService (max 5 connections đã config)
- Schema creation idempotent (CREATE IF NOT EXISTS)
- All queries parameterized (SQL injection safe)
- Graceful degradation: KB failure doesn't block investigation pipeline

## Architecture

```
AppState
  ├── cache: Option<Arc<CacheService>>        ← giữ nguyên
  └── knowledge_base: Option<Arc<KnowledgeBase>>  ← MỚI, share pool

KnowledgeBase
  ├── ensure_schema()              → CREATE IF NOT EXISTS
  ├── upsert_subject()             → INSERT ON CONFLICT UPDATE
  ├── insert_investigation()       → INSERT RETURNING id
  ├── insert_evidence()            → INSERT (bulk)
  ├── upsert_subject_link()        → INSERT ON CONFLICT UPDATE strength
  ├── get_subject_history()        → subject + investigations + evidence + links + reports
  ├── recalculate_risk()           → UPDATE subjects SET risk_score = ...
  ├── refresh_materialized_view()  → REFRESH CONCURRENTLY
  └── start_refresh_task()         → tokio::spawn interval 15 min
```

## Related Code Files

### Files to Create
- `src/knowledge_base/mod.rs` — KnowledgeBase service struct + schema + CRUD
- `src/knowledge_base/models.rs` — Subject, InvestigationRecord, Evidence, SubjectLink, UserReport structs
- `src/knowledge_base/risk.rs` — risk recalculation logic

### Files to Modify
- `src/main.rs` — add `mod knowledge_base`
- `src/api/mod.rs` — add `knowledge_base: Option<Arc<KnowledgeBase>>` to AppState, init in `build_state()`
- `src/config.rs` — no change needed (reuse DATABASE_URL)

## Implementation Steps

### 1. Create `src/knowledge_base/models.rs`
Define structs matching DB schema:
```rust
pub struct Subject {
    pub id: Uuid,
    pub value: String,
    pub subject_type: String, // "phone", "bank", "url"
    pub risk_score: f32,
    pub risk_level: String,
    pub report_count: i32,
    pub investigation_count: i32,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

pub struct SubjectHistory {
    pub subject: Subject,
    pub recent_investigations: Vec<InvestigationRecord>,
    pub approved_reports: Vec<UserReportRecord>,
    pub linked_subjects: Vec<LinkedSubject>,
    pub all_risk_signals: Vec<String>,
}

pub struct InvestigationRecord {
    pub id: Uuid,
    pub risk_level: String,
    pub risk_score_numeric: f32,       // 0.0-1.0
    pub confidence: f32,
    pub sources_analyzed: i32,
    pub sources_success_ratio: f32,    // successful/total
    pub quality_score: f32,            // weighted quality
    pub created_at: DateTime<Utc>,
}

pub struct LinkedSubject {
    pub subject: Subject,
    pub link_type: String,
    pub strength: f32,
}

pub struct UserReportRecord {
    pub id: Uuid,
    pub description: String,
    pub category: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}
```

### 2. Create `src/knowledge_base/mod.rs`
- `pub mod models; pub mod risk;`
- `KnowledgeBase` struct wrapping `PgPool`
- `new(pool: PgPool)` — share pool from CacheService
- `ensure_schema()` — CREATE TABLE IF NOT EXISTS for all 5 tables + indexes + materialized view
- `upsert_subject(value, subject_type)` → `INSERT ... ON CONFLICT(value, subject_type) DO UPDATE SET last_seen_at = NOW(), investigation_count = investigation_count + 1 RETURNING id`
- `insert_investigation(subject_id, query, query_type, risk_level, risk_score_numeric, confidence, sources_analyzed, sources_success_ratio, quality_score, duration_ms, detective_report)` → `INSERT RETURNING id`
- `insert_evidence_batch(items: Vec<EvidenceInput>)` — bulk insert with unnest or loop
- `upsert_subject_link(subject_a_id, subject_b_id, link_type, evidence_id)` — ensure a < b ordering, ON CONFLICT increment strength, append evidence_id
- `get_subject_by_value(value, subject_type)` → `Option<Subject>`
- `get_subject_history(subject_id, limit)` → `SubjectHistory` — single query with JOINs or separate queries
- `start_refresh_task(pool)` — tokio::spawn, interval 15 min, `REFRESH MATERIALIZED VIEW CONCURRENTLY`

### 3. Create `src/knowledge_base/risk.rs`
- `compute_quality_score(sources_success_ratio, confidence, risk_signals_count, evidence_urls_count)` — returns f32 (0.0-1.0)
  - Formula: `sources_success_ratio × 0.4 + confidence × 0.3 + evidence_richness × 0.2 + recency × 0.1`
  - evidence_richness = `min(1.0, (risk_signals + evidence_urls) / 10.0)`
  - recency = 1.0 at ingest time (applied during aggregation)
- `risk_level_to_numeric(risk_level: &str)` → f32: critical=1.0, high=0.8, medium=0.5, low=0.2, unknown=0.0
- `numeric_to_risk_level(score: f32)` → &str: >=0.8 critical, >=0.6 high, >=0.3 medium, >0 low, 0 unknown
- `recalculate_risk(pool, subject_id)` — weighted average of all investigations:
  ```sql
  UPDATE subjects SET risk_score = (
    SELECT COALESCE(
      SUM(i.risk_score_numeric * i.quality_score) / NULLIF(SUM(i.quality_score), 0),
      0
    ) FROM investigations i WHERE i.subject_id = $1
  ) WHERE id = $1
  ```
  Then map risk_score → risk_level using `numeric_to_risk_level()`

### 4. Update `src/main.rs`
- Add `mod knowledge_base;`

### 5. Update `src/api/mod.rs`
- Add `knowledge_base` field to `AppState`
- In `build_state()`: if database_url exists, create `KnowledgeBase` from same pool
- Extract pool from CacheService → share with KnowledgeBase
  - Refactor: `CacheService` exposes `pub fn pool(&self) -> &PgPool`
  - Or: create pool once in `build_state()`, pass to both CacheService and KnowledgeBase

### 6. Schema SQL (reference for ensure_schema)
See brainstorm report for full SQL. Key points:
- `subjects` — UNIQUE(value, subject_type), indexes on value, type, risk
- `investigations` — FK to subjects, index on subject_id, created_at DESC
- `evidence` — FK to subjects + investigations, GIN index on data + mentioned_subjects
- `subject_links` — CHECK(subject_a_id < subject_b_id), UNIQUE(a, b, link_type)
- `user_reports` — FK to subjects, index on status
- `subject_risk_overview` — materialized view, UNIQUE index on id for CONCURRENTLY refresh

## Todo List
- [x] Create `src/knowledge_base/models.rs` with all struct definitions
- [x] Create `src/knowledge_base/risk.rs` with recalculation logic
- [x] Create `src/knowledge_base/mod.rs` with KnowledgeBase service
- [x] Implement `ensure_schema()` — all 5 tables + indexes + materialized view
- [x] Implement CRUD: upsert_subject, insert_investigation, insert_evidence_batch
- [x] Implement upsert_subject_link with ordering constraint
- [x] Implement get_subject_by_value + get_subject_history
- [x] Implement start_refresh_task (15 min interval)
- [x] Refactor pool sharing: create shared pool once in `build_state()`
- [x] Add `knowledge_base` to AppState and init in build_state()
- [x] Add `mod knowledge_base` to main.rs
- [x] Compile check — `cargo check`

## Success Criteria
- All 5 tables created on startup (idempotent)
- KnowledgeBase service compiles, available in AppState
- Can upsert subjects, insert investigations/evidence, create links
- Materialized view auto-refreshes every 15 min
- Pool shared between CacheService and KnowledgeBase
- KB failure doesn't crash server (graceful degradation)

## Risk Assessment
- **Pool contention:** 5 max connections shared → may need increase to 8-10 if KB writes heavy. Monitor connection wait times.
- **Schema migration on existing DB:** Using CREATE IF NOT EXISTS → safe for existing deployments. No DROP statements.

## Security Considerations
- All queries use parameterized binds (sqlx)
- No raw SQL interpolation
- User reports: IP stored as SHA256 hash only
