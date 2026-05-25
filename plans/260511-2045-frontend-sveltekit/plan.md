---
status: complete
created: 2026-05-11
title: Frontend SvelteKit Implementation
description: SvelteKit frontend for scam investigation platform with SSE streaming, dark mode, shareable URLs
---

# Frontend SvelteKit Implementation

## Overview

Build SvelteKit frontend consuming Rust backend's SSE `/api/investigate` endpoint. Google-style search → streaming investigation results → detective narrative.

## Context

- **Brainstorm:** `plans/reports/brainstorm-260511-2045-frontend-architecture.md`
- **Backend SSE:** `src/api/investigate.rs` — `GET /api/investigate?q={query}&type={phone|bank|url}`
- **Stack:** SvelteKit 2, Svelte 5, Tailwind CSS 4, marked
- **Backend:** Rust/Axum on port 3000

## SSE Events (from backend)

| Event | Fields | Purpose |
|-------|--------|---------|
| `phase_start` | phase, label, total_sources? | Phase began |
| `source_status` | source, status, found | Source scraping done |
| `progress` | phase, message | Agent working |
| `summary_result` | source, result{summary,key_facts,risk_signals} | Source analysis done |
| `url_assessment` | selected, total, urls[] | URL selection done |
| `extraction_result` | url, result{summary,entities,risk_signals} | URL extraction done |
| `detective_stream` | chunk, done, replace | Streaming detective narrative |
| `complete` | risk_level, confidence, sources_analyzed, duration_ms | Investigation done |
| `error` | phase?, message, recoverable | Error occurred |

## Phases

| # | Phase | Priority | Effort | Status |
|---|-------|----------|--------|--------|
| 1 | [Project Bootstrap](phase-01-bootstrap.md) | Critical | S | complete |
| 2 | [Search + SSE Consumer](phase-02-search-sse.md) | Critical | M | complete |
| 3 | [Results UI](phase-03-results-ui.md) | Critical | M | complete |
| 4 | [Detective Report + Dark Mode](phase-04-detective-darkmode.md) | High | M | complete |
| 5 | [SEO + Polish](phase-05-seo-polish.md) | Medium | S | complete |

## Dependencies

```
Phase 1 → Phase 2 (needs project skeleton)
Phase 2 → Phase 3 (results need SSE data)
Phase 3 → Phase 4 (detective needs results context)
Phase 4 → Phase 5 (polish needs working UI)
```

## Key Decisions

- **SvelteKit server proxy** — Frontend proxies SSE to backend, avoids CORS
- **Svelte 5 runes** — Use `$state`, `$derived`, `$effect` (not legacy stores)
- **Tailwind 4** — CSS-first config, `@theme` directive, no PostCSS plugin
- **marked** — Markdown rendering for detective output, minimal dep
- **URL state** — Query in URL params `?q=...&type=...` for sharing/bookmarking
