---
title: "Persistent Knowledge Base"
description: "Add persistent scam knowledge base with hybrid PostgreSQL model, pipeline integration, link detection, and crowdsource support"
status: complete
priority: P1
effort: 20h
branch: main
tags: [backend, database, feature, api]
created: 2026-05-23
---

# Persistent Knowledge Base

## Overview

Chuyển từ cache ephemeral (TTL 1h) sang persistent knowledge base. Tích lũy dữ liệu điều tra qua thời gian, enrich investigations với historical data, phát hiện liên kết giữa subjects, và hỗ trợ crowdsource reports.

**Progress:** 100% — backend/frontend implementation completed and DB-backed validation evidence collected on 2026-05-24.

**Approach:** Hybrid PostgreSQL (relational core + JSONB evidence + materialized views)

**Brainstorm:** [brainstorm report](../reports/brainstorm-260523-2259-persistent-knowledge-base.md)

## Phases

| # | Phase | Status | Effort | Link |
|---|-------|--------|--------|------|
| 1 | Schema & KnowledgeBase service | Complete (100%) | 6h | [phase-01](./phase-01-schema-and-knowledge-base-service.md) |
| 2 | Pipeline integration (ingest + enrich) | Complete (100%) | 6h | [phase-02](./phase-02-pipeline-integration.md) |
| 3 | Link detection & network API | Complete (100%) | 4h | [phase-03](./phase-03-link-detection-and-network-api.md) |
| 4 | Crowdsource & admin API | Complete (100%) | 4h | [phase-04](./phase-04-crowdsource-and-admin-api.md) |

## Dependencies

- PostgreSQL already in stack (DATABASE_URL)
- Cache layer (analysis_cache, investigation_cache) **giữ nguyên** — không thay đổi
- Phase 2 depends on Phase 1
- Phase 3 depends on Phase 2 (needs ingest to populate links)
- Phase 4 independent of Phase 3 (can parallel after Phase 1)

## Key Decisions

- **Ingest gate:** Only `quality_score >= 0.3` (retain useful investigations, including low-risk confirmations with strong evidence)
- **Evidence schema:** JSONB flexible — no migration khi thêm scraper/agent mới
- **Links:** Undirected (CHECK subject_a_id < subject_b_id), strength = co-occurrence count
- **Materialized view:** Periodic refresh (15 min), CONCURRENTLY (no read lock)
- **User reports:** Anonymous + admin approval gate
