# Codebase Summary

> Last updated: 2026-05-25
> Source snapshot: `repomix-output.xml`

## Overview

Tra Cuu Lua Dao is a two-part application:
- Rust/Axum backend for investigation orchestration, SSE delivery, cache, and persistent knowledge base
- SvelteKit frontend for the narrative investigation UI, buffered report replay, and community-report UX

Repomix snapshot summary:
- Packed files: 178
- Total tokens: 304,275
- Primary large files: `frontend/src/routes/+page.svelte`, `src/pipeline/investigation.rs`, `src/bin/checkscam_crawler.rs`

## Major Areas

### Backend

- `src/lib.rs` now exposes the shared crate surface so the web server and auxiliary binaries can reuse the same modules.
- `src/main.rs` is the Axum bootstrap only.
- `src/bin/checkscam_crawler.rs` adds a standalone bulk-ingest path for CheckScam WordPress content.
- `src/api/` exposes health, investigation SSE, report replay SSE, subject lookup, subject network, community reports, and admin moderation routes.
- `src/pipeline/` contains the multi-phase investigation flow plus helper logic for loading historical context and persisting investigation output into the knowledge base.
- `src/scrapers/` holds the source-specific collectors and proxy-aware HTTP clients, including direct-source scrapers such as `admin.vn` and browser-assisted sources such as `checkscam.vn`.
- `src/knowledge_base/` implements durable scam intelligence storage on PostgreSQL, including crawler-specific media metadata and post-crawl linking helpers.
- `src/cache/` remains separate from the knowledge base and keeps TTL-based cached scraper and agent outputs.
- `src/report_store.rs` manages Redis-backed detective report replay.

### Frontend

- `frontend/src/routes/+page.svelte` is the main experience and currently contains most investigation, replay, historical context, linked-subject, and community-report UI state.
- `frontend/src/lib/api-base.ts` resolves frontend API endpoints from `VITE_API_BASE_URL` with a relative `/api` fallback.
- `frontend/src/lib/sse-client.ts` handles both live investigation SSE and post-completion report replay SSE.
- `frontend/src/lib/types.ts` mirrors frontend event and graph payload shapes from backend APIs.
- `frontend/src/lib/markdown.ts` sanitizes and renders the detective markdown output.

### Operations

- `docker-compose.yml` defines `frontend`, `backend`, `postgres`, and `redis`.
- production web/API routing is split by domain, with frontend using `VITE_API_BASE_URL` to reach the backend.
- `deploy/systemd/` includes host-side service files for model servers and a backend service.

## Persistent Knowledge Base Feature

The new persistent knowledge base is implemented across:
- `src/knowledge_base/schema.rs`
- `src/knowledge_base/media.rs`
- `src/knowledge_base/subjects.rs`
- `src/knowledge_base/reports.rs`
- `src/knowledge_base/risk.rs`
- `src/pipeline/knowledge_base.rs`
- `src/api/subjects.rs`
- `src/api/reports.rs`
- `src/api/admin.rs`
- `frontend/src/routes/+page.svelte`

Key behavior:
- subjects are normalized and persisted by `value + subject_type`
- repeated investigations can emit `historical_context`
- completed investigations are ingested asynchronously when `quality_score >= 0.3`
- evidence rows store flexible JSONB agent outputs
- links are built from extracted mentions and surfaced through `/api/subjects/{value}/network`
- anonymous community reports are stored as pending and require admin approval to affect aggregate risk
- media metadata is stored in a polymorphic `media` table while file bytes live under `data/media/`

## Bulk Crawler And Media Ingest

The crawler/media refactor spans:
- `src/bin/checkscam_crawler.rs`
- `src/lib.rs`
- `src/main.rs`
- `src/knowledge_base/media.rs`
- `src/knowledge_base/models.rs`
- `src/knowledge_base/schema.rs`
- `src/knowledge_base/subjects.rs`

Observed behavior from the current implementation:
- `checkscam-crawler` pages through the CheckScam WordPress REST API with configurable concurrency, delay, resume mode, and dry-run mode
- it reuses `scrapers::checkscam::parse_detail_report()` and optionally hits the browser sidecar for detail HTML
- crawled posts create `external_report` evidence rows with `source = "checkscam_crawl"` and `investigation_id = NULL`
- image handling is deduplicated first per evidence/original URL, then globally by `original_url`
- downloaded files are named by SHA-256 hash and written below `data/media/evidence/{evidence_id}/`
- a second pass creates `co_mentioned` subject links and reapplies crawler-derived risk floors

## Current Architectural Shape

### Ephemeral vs persistent storage

| Layer | Backing store | Purpose |
|-------|---------------|---------|
| Cache | PostgreSQL | 24h/1h TTL data for scraper and agent output reuse |
| Report replay | Redis | buffered detective markdown chunks |
| Knowledge base | PostgreSQL | durable subject history, investigations, evidence, links, community reports, and media metadata |
| Evidence files | Filesystem (`data/media/`) | downloaded crawler media bytes referenced by `media.file_path` |

### Public API groups

| Group | Routes |
|-------|--------|
| Investigation | `/health`, `/api/investigate`, `/api/investigate/report` |
| Knowledge base | `/api/subjects/{value}`, `/api/subjects/{value}/network` |
| Community reports | `/api/reports` |
| Admin moderation | `/api/admin/reports`, `/api/admin/reports/{id}/approve`, `/api/admin/reports/{id}/reject` |

## Observations

- The backend degrades gracefully when PostgreSQL or Redis is unavailable, but knowledge-base-driven UX becomes partial or unavailable.
- The backend is now a library-plus-binaries layout; new operational behavior can ship without bloating the web server entry point.
- If operators run the crawler inside the current Compose stack, `docker-compose.yml` does not yet mount a dedicated persistent volume for `data/media`.
- The main frontend route file is large and owns many concerns; it is the dominant UI context file in the repo.
- `ADMIN_API_KEY` is wired through backend config, Compose deployment, and admin moderation routes.
