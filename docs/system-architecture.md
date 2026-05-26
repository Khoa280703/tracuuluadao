# System Architecture

> Last updated: 2026-05-25

## Overview

Tra Cuu Lua Dao is a multi-source scam investigation platform. Users submit phone numbers, bank account numbers, or URLs, and the system queries Vietnamese anti-scam sources, synthesizes findings with LLM-powered agents, and returns a narrative detective report with a risk score through SSE.

The platform now has two data-ingest paths:
- **Live investigation pipeline** for on-demand user queries over SSE
- **Offline bulk crawler** for seeding the knowledge base from CheckScam WordPress content

The storage model now spans three layers:
- **Ephemeral cache** in PostgreSQL for scraper results and agent outputs with TTL
- **Persistent knowledge base** in PostgreSQL for subjects, investigations, evidence, links, community reports, and media metadata
- **Filesystem media store** under `data/media/` for downloaded evidence files referenced by the knowledge base

## High-Level Architecture

```
┌─────────────────────┐        ┌──────────────────────────┐
│ Frontend (SvelteKit)│        │ Backend (Rust/Axum)      │
│                     │  SSE   │                          │
│ /                   │◄──────►│ /api/investigate         │
│ narrative UI        │  HTTP  │ /api/investigate/report  │
│ history panel       │◄──────►│ /api/subjects/*          │
│ linked subjects     │◄──────►│ /api/reports             │
│ community reports   │◄──────►│ /api/admin/reports/*     │
└─────────────────────┘        └──────────┬───────────────┘
                                          │
                      ┌───────────────────┼───────────────────┐
                      ▼                   ▼                   ▼
               PostgreSQL            Redis Streams        Qwen APIs
            cache + knowledge         report replay      summarizer /
             base + media rows                            extractor /
                                                          detective
                      ▲
                      │
          ┌───────────┴───────────┐
          │ checkscam-crawler CLI │
          │ REST crawl + media DL │
          └───────────┬───────────┘
                      │
                      ▼
                 data/media/
               evidence files
```

## Backend Architecture

### Technology Stack

| Component | Technology |
|-----------|------------|
| Runtime | Rust 2024 edition |
| HTTP framework | Axum 0.8 |
| Async runtime | Tokio |
| Database | PostgreSQL via `sqlx` |
| Report buffer | Redis Streams |
| SSE | axum `Sse` + `async-stream` |
| Logging | `tracing` + `tracing-subscriber` |

### Module Structure

```
src/
├── lib.rs              # shared library surface for web server + crawler binary
├── main.rs
├── bin/
│   └── checkscam_crawler.rs # bulk crawler CLI
├── api/
│   ├── admin.rs         # /api/admin/reports*
│   ├── health.rs        # /health
│   ├── investigate.rs   # /api/investigate + /api/investigate/report
│   ├── mod.rs           # router + AppState
│   ├── reports.rs       # /api/reports
│   └── subjects.rs      # /api/subjects/*
├── cache/
│   └── mod.rs           # TTL cache tables + cleanup task
├── knowledge_base/
│   ├── mod.rs           # KnowledgeBase + view refresh task
│   ├── media.rs         # media CRUD + dedup helpers
│   ├── models.rs        # Subject, history, graph, report models
│   ├── reports.rs       # report rate limit + moderation queries
│   ├── risk.rs          # quality score + aggregate risk recalculation
│   ├── schema.rs        # persistent KB schema bootstrap
│   └── subjects.rs      # subject, investigation, evidence, graph queries
├── pipeline/
│   ├── investigation.rs # main investigation orchestration
│   ├── knowledge_base.rs# preload history + async ingest helpers
│   ├── state.rs         # InvestigationEvent payloads
│   └── url_fetcher.rs
├── report_store.rs      # Redis-backed report replay store
└── scrapers/
    ├── admin_vn.rs
    ├── checkscam.rs
    ├── chongluadao.rs
    ├── duckduckgo.rs
    ├── google.rs
    ├── http_client.rs
    ├── mod.rs
    ├── proxy.rs
    ├── tinnhiemmang.rs
    └── trangtrang.rs
```

### Library Split And Binaries

- `src/lib.rs` exports the shared modules so both the Axum server and the crawler can reuse the same cache, scraper, logging, and knowledge-base code.
- `src/main.rs` is reduced to application bootstrap for the HTTP server.
- `src/bin/checkscam_crawler.rs` is an operational binary, not an API route. It seeds persistent data and evidence media without going through the SSE pipeline.

### Request Lifecycle

```
1. Browser opens `${VITE_API_BASE_URL}/api/investigate` SSE
2. Backend emits investigation_started
3. Optional subject history is loaded from knowledge base
4. Pipeline runs 5 investigation phases
5. Detective chunks stream to Redis-backed replay store when available
6. Complete event closes the process SSE
7. Frontend opens `${VITE_API_BASE_URL}/api/investigate/report` for buffered markdown replay
8. Investigation result is ingested into the knowledge base asynchronously
9. Frontend loads `${VITE_API_BASE_URL}/api/subjects/{value}/network` for linked-subject cards
```

### Investigation Pipeline

#### Historical preload

- `pipeline::knowledge_base::load_subject_history()` checks whether the normalized subject already exists.
- If found, backend emits `historical_context` before the main phase work finishes.
- The same historical data is serialized into the detective prompt under `historical_context`.

#### Phase 1: Data collection

- Scrapers run in parallel with a 10-second budget per scraper.
- Dedicated anti-scam sources now include `admin.vn`, `checkscam.vn`, `chongluadao.vn`, `tinnhiemmang.vn`, and `trangtrang.com`.
- Google is primary search; DuckDuckGo is fallback.
- Cache is checked before scraper execution when cache service is available.

#### Phase 2: Source summarization

- `summarizer` uses Qwen 3.5 output schemas.
- Output includes `key_facts`, `phone_mentions`, `risk_signals`, and evidence URLs.

#### Phase 3: URL selection

- `url-assessor` selects the most relevant URLs for deeper extraction.
- Fallback is the first search results when model output is unavailable.

#### Phase 4: URL deep analysis

- `extractor` fetches and parses selected pages.
- Output includes `entities`, `risk_signals`, and `related_numbers`.

#### Phase 5: Detective report

- `detective` streams markdown response chunks.
- Final chunk sequence is buffered into Redis when `REDIS_URL` is configured.
- Frontend intentionally separates the process narrative from the final report body.

#### Post-completion ingest

- The pipeline spawns an async write-back task after the result is assembled.
- Ingest is skipped when `quality_score < 0.3`.
- Stored investigation data updates subject risk, creates evidence rows, and detects subject links from mentioned values.

## Bulk Crawler Pipeline

The `checkscam-crawler` binary is a separate ingestion path for historical seed data:

1. Fetch WordPress posts from `https://checkscam.vn/wp-json/wp/v2/posts`
2. Page through results with bounded concurrency and per-page delay
3. Fetch detail HTML from the sidecar when reachable, otherwise fall back to `content.rendered`
4. Extract phone numbers, bank accounts, warning text, report count, and CheckScam-hosted upload images
5. Upsert subjects and insert `external_report` evidence with `source = "checkscam_crawl"`
6. Download or reuse images, then insert `media` rows pointing at filesystem paths
7. Run a second pass over `mentioned_subjects` to create `co_mentioned` links and reapply crawler risk floors

Idempotency behavior in the current implementation:
- `--resume` skips posts when matching evidence and expected media already exist
- media dedup first checks `(entity_type, entity_id, original_url)`, then falls back to global `original_url` reuse
- downloaded files are content-addressed by SHA-256 filename under `data/media/evidence/{evidence_id}/`

## Persistent Knowledge Base

### Schema

The backend bootstraps these tables and indexes automatically through `KnowledgeBase::ensure_schema()`:

| Table | Purpose |
|-------|---------|
| `subjects` | Canonical subject rows keyed by normalized `value + subject_type` |
| `investigations` | Durable record of completed investigations and final detective markdown |
| `evidence` | JSONB-backed summaries and extractions with risk signals and mentioned subjects |
| `subject_links` | Undirected links between related subjects with accumulated `strength` |
| `user_reports` | Anonymous community reports with moderation status |
| `media` | File metadata for downloaded evidence assets, keyed by polymorphic `entity_type + entity_id` |

Additional read model:
- `subject_risk_overview` materialized view
- refreshed on startup and every 15 minutes by a background refresh task

### Subject normalization

Normalization rules are reused across read/write paths:
- `phone`: keep ASCII digits only
- `bank`: trim and remove whitespace
- `url`: lowercase and strip trailing slash

### Aggregate risk model

The aggregate subject score combines:
- weighted average of persisted investigation scores using `quality_score`
- approved report count signal capped at `0.72`

Approved community reports therefore affect final `subjects.risk_score`, `subjects.risk_level`, and `subjects.report_count`.

Crawler-specific behavior:
- crawled evidence can raise a subject's minimum crawl-derived risk floor
- after link detection, the highest CheckScam-derived floor is reapplied for touched subjects
- crawl evidence uses `investigation_id = NULL`, so seed data stays separate from user-triggered investigations

### Subject network graph

- Links are stored once per pair using `CHECK(subject_a_id < subject_b_id)`.
- Current link types are `mentioned_together` from live pipeline ingest and `co_mentioned` from crawler post-processing.
- `GET /api/subjects/{value}/network` traverses related nodes with a recursive CTE.
- `depth` is clamped to `1..3`.

### Shared Media Storage

- File bytes are not stored in PostgreSQL; only metadata is persisted in `media`.
- Current writer is the crawler, which stores `entity_type = "evidence"` rows.
- `media.original_url` is nullable and is used for crawl dedup/reuse when the same CheckScam image appears across multiple evidence records.
- `file_path` is stored relative to the application working directory, matching paths under `data/media/`.

## API Surface

### SSE investigation APIs

| Method | Path | Notes |
|--------|------|-------|
| GET | `/api/investigate` | Main process stream |
| GET | `/api/investigate/report` | Report replay stream; requires Redis-backed store |

Important SSE event types:

| Event | Payload summary |
|-------|-----------------|
| `investigation_started` | `{ investigation_id }` |
| `phase_start` | `{ phase, label, total_sources? }` |
| `source_status` | `{ source, status, found }` |
| `summary_result` | `{ source, result }` |
| `url_assessment` | `{ selected, total, urls }` |
| `extraction_result` | `{ url, result }` |
| `historical_context` | `{ investigation_count, last_risk_level, approved_reports, linked_subjects_count, first_seen, last_seen }` |
| `detective_stream` | `{ chunk, done, replace }` |
| `complete` | `{ investigation_id, risk_level, confidence, sources_analyzed, duration_ms }` |
| `investigation_error` | `{ phase?, message, recoverable }` |

### Subject APIs

| Method | Path | Result |
|--------|------|--------|
| GET | `/api/subjects/{value}?type={type}` | Subject history payload |
| GET | `/api/subjects/{value}/network?type={type}&depth={n}` | Subject network graph payload |

Subject history payload contains:
- `subject`
- `recent_investigations`
- `approved_reports`
- `linked_subjects`
- `all_risk_signals`

### Community report APIs

| Method | Path | Result |
|--------|------|--------|
| POST | `/api/reports` | Creates pending report, returns `{ "id": "<uuid>" }` |
| GET | `/api/admin/reports` | Returns `UserReportWithSubject[]` |
| POST | `/api/admin/reports/{id}/approve` | Returns `{ "ok": true, "status": "approved" }` |
| POST | `/api/admin/reports/{id}/reject` | Returns `{ "ok": true, "status": "rejected" }` |

Moderation rules:
- admin endpoints require `x-admin-key`
- missing or invalid key returns unauthorized/config error
- approval and rejection both trigger subject risk recalculation

Abuse controls on `POST /api/reports`:
- max 5 reports per hashed IP in a 24h window
- same hashed IP cannot report the same subject again within 24h
- descriptions are capped at 2000 characters

## Frontend Architecture

### Main frontend responsibilities

- open and manage the investigation SSE connection
- open a second SSE connection for buffered report replay
- render historical context before or alongside live investigation updates
- fetch subject network data after investigation completion
- submit community reports to `/api/reports`
- derive API endpoints from `VITE_API_BASE_URL` when web and API are deployed on separate domains

### Key frontend states

| State | Purpose |
|-------|---------|
| `historicalContext` | Holds `historical_context` SSE payload for the report tab |
| `linkedSubjectsGraph` | Stores graph response from `/api/subjects/{value}/network` |
| `reportDescription`, `reportCategory` | Community report form state |
| `reportSubmitError`, `reportSubmitMessage` | Submission feedback |

### UX behavior tied to the knowledge base

- When historical data exists, the report tab shows a dedicated "Dữ liệu lịch sử từ hệ thống" card.
- After a finished investigation, the right column loads linked subjects and displays their risk labels and graph depth.
- The community report card submits anonymous reports and tells the user moderation is required before the report affects system knowledge.

## Operational Dependencies

### Required for full feature set

| Dependency | Needed for |
|------------|------------|
| PostgreSQL | cache + persistent knowledge base |
| Redis | buffered report replay SSE |
| `ADMIN_API_KEY` | admin moderation APIs |

### Graceful degradation

- Without `DATABASE_URL`, investigations still run but subject history, linked-subject graph, community reports, and moderation APIs are unavailable.
- Without `REDIS_URL`, the final report falls back to direct SSE behavior and replay endpoint is unavailable.
- Without `ADMIN_API_KEY`, report submission still works but admin review APIs remain disabled even though the Compose stack now forwards the variable when provided.
