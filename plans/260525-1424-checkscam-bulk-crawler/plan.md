---
title: "Checkscam Bulk Crawler + Media Architecture"
description: "CLI crawler to seed KB from checkscam.vn with shared media table"
status: completed
priority: P1
effort: 10h
branch: main
tags: [crawler, media, knowledge-base, checkscam, cli]
created: 2026-05-25
---

# Checkscam Bulk Crawler + Media Architecture

## Goal

Bulk-crawl checkscam.vn (WordPress) posts into the existing KB schema. Download evidence images into a new shared media table. CLI binary, idempotent, rate-limited.

## Phases

| # | Phase | Effort | Status |
|---|-------|--------|--------|
| 1 | [Media table schema + CRUD](./phase-01-media-table-schema-crud.md) | 1.5h | completed |
| 2 | [Crawler CLI + REST pagination](./phase-02-crawler-cli-rest-pagination.md) | 3.5h | completed |
| 3 | [Detail parsing + image download + DB insert](./phase-03-detail-parsing-image-db.md) | 3.5h | completed |
| 4 | [Link detection + risk + summary](./phase-04-link-detection-risk-summary.md) | 1.5h | completed |

## Key Dependencies

- PostgreSQL (DATABASE_URL) — existing KB tables must exist
- Browser sidecar at `127.0.0.1:4417` — optional, fallback to REST API content
- Existing: `parse_detail_report()`, `HttpClientFactory`, `KnowledgeBase` service

## Critical Architecture Decision: lib.rs

The crawler is a separate binary that reuses library code. This requires creating `src/lib.rs` and converting `src/main.rs` from `mod` declarations to `use` imports. This is the **highest-risk change** — it touches every module declaration. See Phase 2 for detailed instructions.

**Key visibility issue:** `KnowledgeBase.pool` is currently `pub(crate)`. With `lib.rs`, the binary is a separate crate and cannot access `pub(crate)` items. Solution: add dedicated methods to `KnowledgeBase` (e.g., `evidence_exists_for_url`, `set_crawl_risk`) instead of raw pool access.

## Files to Create

- `src/lib.rs` — library crate root exposing modules for binary
- `src/bin/checkscam_crawler.rs` — CLI entry point
- `src/knowledge_base/media.rs` — media CRUD

## Files to Modify

- `src/main.rs` — switch from `mod` to `use tracuuluadao::*` imports
- `src/knowledge_base/schema.rs` — add media table DDL
- `src/knowledge_base/models.rs` — add MediaRecord model
- `src/knowledge_base/mod.rs` — register media module, add crawler helper methods
- `src/knowledge_base/subjects.rs` — add `evidence_exists_for_url`, `set_crawl_risk` methods
- `src/scrapers/checkscam.rs` — make `parse_detail_report` and `SIDECAR_BASE_URL` pub
- `Cargo.toml` — add `[[bin]]` entry, `clap` and `hex` dependencies

## Risk Assessment

- **lib.rs restructure (HIGH):** Moving all modules to lib.rs is a breaking structural change. Must verify both binaries compile after. Some `use crate::` paths in existing code become `use crate::` in lib context — should work but needs verification.
- **Cloudflare on REST API (LOW):** WP REST rarely behind CF. Fallback: sidecar for all detail fetches.
- **Data volume (LOW):** ~10K posts expected. Memory ~10MB. Runtime: hours with rate limiting.
- **Idempotency (LOW):** Upsert + dedup logic handles re-runs.

## Brainstorm Report

- [brainstorm-260525-1424-checkscam-bulk-crawler.md](../reports/brainstorm-260525-1424-checkscam-bulk-crawler.md)
