---
title: "Checkscam Bulk Crawler + Media Architecture"
description: "CLI crawler to seed KB from checkscam.vn with shared media table"
status: pending
priority: P1
effort: 9h
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
| 1 | [Media table schema + CRUD](./phase-01-media-table-schema-crud.md) | 1.5h | pending |
| 2 | [Crawler CLI + REST pagination](./phase-02-crawler-cli-rest-pagination.md) | 3h | pending |
| 3 | [Detail parsing + image download + DB insert](./phase-03-detail-parsing-image-db.md) | 3h | pending |
| 4 | [Link detection + risk + summary](./phase-04-link-detection-risk-summary.md) | 1.5h | pending |

## Key Dependencies

- PostgreSQL (DATABASE_URL) — existing KB tables must exist
- Browser sidecar at `127.0.0.1:4417` — optional, fallback to REST API content
- Existing: `parse_detail_report()`, `HttpClientFactory`, `KnowledgeBase` service

## Files to Create

- `src/bin/checkscam_crawler.rs` — CLI entry point
- `src/knowledge_base/media.rs` — media CRUD

## Files to Modify

- `src/knowledge_base/schema.rs` — add media table DDL
- `src/knowledge_base/models.rs` — add MediaRecord model
- `src/knowledge_base/mod.rs` — register media module
- `src/scrapers/checkscam.rs` — make `parse_detail_report` pub(crate), extract image URLs
- `Cargo.toml` — add `[[bin]]` entry + clap dependency

## Brainstorm Report

- [brainstorm-260525-1424-checkscam-bulk-crawler.md](../reports/brainstorm-260525-1424-checkscam-bulk-crawler.md)
