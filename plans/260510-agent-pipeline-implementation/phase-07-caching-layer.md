# Phase 7: Caching Layer

## Priority: Medium | Effort: M | Status: completed

## Overview

PostgreSQL cache for scrape results and LLM analysis. Reduces redundant scraping and LLM calls for repeated queries.

## Current Reality

- `src/cache/mod.rs` implements PostgreSQL-backed scrape, analysis, and full-investigation cache APIs.
- App boot calls `ensure_schema()` and `start_cleanup_task()`, so tables/indexes self-bootstrap and expired rows are cleaned on startup plus every 6h.
- App also boots without `DATABASE_URL`; cache is disabled cleanly and `/health` reports `cache_enabled:false`.
- `src/scrapers/mod.rs` wires per-source scrape cache, while `src/pipeline/investigation.rs` wires analysis and full-investigation cache.
- Full-investigation cache writes are awaited before return to avoid the repeat-query race seen during live validation.

## Requirements

- Cache scrape results per source (TTL: 24h)
- Cache LLM analysis (TTL: 1h, invalidate on prompt change)
- Cache lookup before pipeline execution
- Deterministic cache write for full-investigation result before returning response
- Cache keys:
  - scrape: `(query_type, query, source)`
  - analysis: `(query, agent_name, prompt_hash, input_hash)`
  - investigation: `(query_type, query, prompt_hash)`

## Architecture

```
src/
├── cache/
│   ├── mod.rs           # CacheService trait + implementation
│   ├── models.rs        # DB models (sqlx)
│   └── migrations/      # SQL migrations
```

### Database Schema

```sql
CREATE TABLE scrape_cache (
    id BIGSERIAL PRIMARY KEY,
    query TEXT NOT NULL,
    query_type TEXT NOT NULL,       -- phone, bank, url
    source TEXT NOT NULL,           -- checkscam, chongluadao, etc.
    result JSONB NOT NULL,          -- ScrapedResult serialized
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_scrape_cache_lookup ON scrape_cache(query_type, query, source, expires_at);

CREATE TABLE analysis_cache (
    id BIGSERIAL PRIMARY KEY,
    query TEXT NOT NULL,
    agent_name TEXT NOT NULL,       -- summarizer, detective, etc.
    prompt_hash TEXT NOT NULL,      -- sha256 of system prompt (for invalidation)
    input_hash TEXT NOT NULL,       -- sha256 of user content
    result JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_analysis_cache_lookup ON analysis_cache(query, agent_name, prompt_hash, input_hash, expires_at);

CREATE TABLE investigation_cache (
    id BIGSERIAL PRIMARY KEY,
    query TEXT NOT NULL,
    query_type TEXT NOT NULL,
    prompt_hash TEXT NOT NULL,       -- sha256 of ALL agent prompts combined (invalidate on any prompt change)
    risk_level TEXT,
    full_result JSONB NOT NULL,     -- Complete InvestigationResult
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_investigation_lookup ON investigation_cache(query, query_type, prompt_hash, expires_at);
```

## Implementation Steps

### 1. Cache Service (`mod.rs`)
```rust
pub struct CacheService {
    pool: PgPool,
}

impl CacheService {
    pub async fn get_scrape(&self, query_type: &str, query: &str, source: &str) -> Option<ScrapedResult>
    pub async fn set_scrape(&self, query_type: &str, query: &str, source: &str, result: &ScrapedResult, ttl: Duration)
    pub async fn get_analysis(&self, query: &str, agent: &str, prompt_hash: &str, input_hash: &str) -> Option<String>
    pub async fn set_analysis(&self, query: &str, agent: &str, prompt_hash: &str, input_hash: &str, result: &str, ttl: Duration)
    pub async fn get_full_investigation(&self, query: &str, query_type: &str, prompt_hash: &str) -> Option<InvestigationResult>
    pub async fn set_full_investigation(&self, query: &str, query_type: &str, prompt_hash: &str, result: &InvestigationResult, ttl: Duration)
}
```

### 2. Integration with Pipeline
- Before Phase 1: check `investigation_cache` → if hit, stream cached result
- Before each scraper: check `scrape_cache` → skip if fresh
- Before each LLM call: check `analysis_cache` → skip if prompt unchanged
- After pipeline: write full result to `investigation_cache` before returning so the next identical query can reuse it immediately
- Per-source and per-agent cache writes stay best-effort; full-investigation write completes before final return

### 3. Prompt Hash for Invalidation
- On agent config load: compute sha256 of full system prompt (includes shared files)
- Store hash in AgentConfig
- On hot-reload: recompute hash → old cache entries auto-invalidate (different prompt_hash won't match)

### 4. TTL Strategy
| Cache | TTL | Rationale |
|-------|-----|-----------|
| scrape_cache | 24h | Scam sites update daily at most |
| analysis_cache | 1h | Short because prompt iteration invalidates anyway |
| investigation_cache | 1h | Full result changes if any sub-cache expires |

### 5. Cleanup
- Background cron: DELETE expired entries every 6h
- Or: PostgreSQL TTL via `pg_cron` extension

## Success Criteria

- [x] Scrape cache lookup/write path is implemented with 24h TTL
- [x] Full investigation cache lookup/write path is implemented
- [x] Full investigation cache write is persisted before response returns
- [x] Second query for same phone verified cached in this pass
- [x] Prompt change → analysis/investigation cache miss → LLM re-runs
- [x] Cache miss on one source → only that source scraped fresh
- [x] Expired entries cleaned up automatically

## Risk Assessment

- **Cache stampede** — Multiple concurrent requests for same uncached query → lock or "cache-aside" pattern
- **JSONB storage limits** — Large detective narratives ~5KB each, manageable
- **PostgreSQL connection pool** — sqlx default pool size = 10, may need tuning
