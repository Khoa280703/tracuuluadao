# Codebase Summary

> Last updated: 2026-05-24
> Source snapshot: `repomix-output.xml`

## Overview

Tra Cuu Lua Dao is a two-part application:
- Rust/Axum backend for investigation orchestration, SSE delivery, cache, and persistent knowledge base
- SvelteKit frontend for the narrative investigation UI, buffered report replay, and community-report UX

Repomix snapshot summary:
- Packed files: 167
- Total tokens: 270,302
- Primary large files: `frontend/src/routes/+page.svelte`, `src/pipeline/investigation.rs`, `scripts/google-browser-sidecar/browser-search.mjs`

## Major Areas

### Backend

- `src/api/` exposes health, investigation SSE, report replay SSE, subject lookup, subject network, community reports, and admin moderation routes.
- `src/pipeline/` contains the multi-phase investigation flow plus helper logic for loading historical context and persisting investigation output into the knowledge base.
- `src/scrapers/` holds the source-specific collectors and proxy-aware HTTP clients.
- `src/knowledge_base/` implements durable scam intelligence storage on PostgreSQL.
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

## Current Architectural Shape

### Ephemeral vs persistent storage

| Layer | Backing store | Purpose |
|-------|---------------|---------|
| Cache | PostgreSQL | 24h/1h TTL data for scraper and agent output reuse |
| Report replay | Redis | buffered detective markdown chunks |
| Knowledge base | PostgreSQL | durable subject history, evidence, links, and community reports |

### Public API groups

| Group | Routes |
|-------|--------|
| Investigation | `/health`, `/api/investigate`, `/api/investigate/report` |
| Knowledge base | `/api/subjects/{value}`, `/api/subjects/{value}/network` |
| Community reports | `/api/reports` |
| Admin moderation | `/api/admin/reports`, `/api/admin/reports/{id}/approve`, `/api/admin/reports/{id}/reject` |

## Observations

- The backend degrades gracefully when PostgreSQL or Redis is unavailable, but knowledge-base-driven UX becomes partial or unavailable.
- The main frontend route file is large and owns many concerns; it is the dominant UI context file in the repo.
- `ADMIN_API_KEY` is wired through backend config, Compose deployment, and admin moderation routes.
