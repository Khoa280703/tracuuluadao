---
status: completed
created: 2026-05-10
title: Agent Pipeline Implementation
description: Implement Rust backend with agent orchestration for scam investigation pipeline
---

# Agent Pipeline Implementation Plan

## Overview

Implement the full Rust backend for tracuuluadao: parallel scraping → LLM agent analysis → SSE streaming to frontend. Self-hosted Qwen models via vLLM.

## Current Reality

- Verified 2026-05-11: `cargo fmt`, `cargo check`, `cargo check --features tls-impersonation`, and `cargo test` all pass.
- Direct model validation and full live pipeline validation are recorded in `plans/reports/validation-260511-203629-agent-pipeline-live.md`.
- `tls-impersonation` now builds and runs through real `rquest` wiring; app boots with and without `DATABASE_URL`.
- Scrape cache, analysis cache, full-investigation cache, startup cleanup, and prompt-hash invalidation are all validated against a real temporary PostgreSQL instance.

## Context

- **Brainstorm report:** `plans/reports/brainstorm-260510-agent-pipeline-architecture.md`
- **Scraping strategy:** `plans/reports/research-260509-2046-google-scraping-strategy.md`
- **LLM endpoints:** Qwen3.5-4B (port 8102), Qwen3.6-27B (port 8002)
- **Stack:** Rust (Axum), PostgreSQL, rquest (TLS impersonation), reqwest (LLM client), tokio

## Phases

| # | Phase | Priority | Effort | Status |
|---|-------|----------|--------|--------|
| 1 | [Project Bootstrap](phase-01-project-bootstrap.md) | Critical | S | completed |
| 2 | [Agent Config System](phase-02-agent-config-system.md) | Critical | M | completed |
| 3 | [LLM Client](phase-03-llm-client.md) | Critical | M | completed |
| 4 | [Scraping Module](phase-04-scraping-module.md) | Critical | L | completed |
| 5 | [Pipeline Orchestration](phase-05-pipeline-orchestration.md) | Critical | L | completed |
| 6 | [SSE Streaming Endpoint](phase-06-sse-streaming.md) | High | M | completed |
| 7 | [Caching Layer](phase-07-caching-layer.md) | Medium | M | completed |
| 8 | [Agent Prompts & Testing](phase-08-agent-prompts.md) | High | M | completed |

## Dependencies

```
Phase 1 → Phase 2, 3, 4 (all need project skeleton)
Phase 2 + 3 → Phase 5 (orchestration needs config + LLM client)
Phase 4 → Phase 5 (orchestration needs scrapers)
Phase 5 → Phase 6 (SSE needs pipeline)
Phase 5 → Phase 7 (caching wraps pipeline)
Phase 2 → Phase 8 (prompts need config structure)
```

## Key Decisions

- **No function calling** — LLM returns JSON structured output, Rust parses + executes
- **Direct vLLM API** — No Python middleware, Rust `reqwest` calls OpenAI-compatible API
- **Hot-reload configs** — `notify` crate watches `config/agents/`, no restart needed
- **Graceful degradation** — If LLM fails, return raw scraped data to user

## Completion Evidence

1. `scripts/validate-agent-pipeline-live.sh` now validates:
   - direct 4B JSON response is parseable with `enable_thinking=false`
   - direct 27B stream returns real text with `enable_thinking=false`
   - 4 parallel 4B calls all return parseable JSON
   - `/api/investigate` completes for `phone`, `bank`, and `url`
2. The same harness proves:
   - phone first run emits detective output with `RISK_LEVEL:` and `CONFIDENCE:`
   - first phone run stays under 60s
   - second identical phone query hits full-investigation cache
   - deleting one scrape source refreshes only that source
   - startup cleanup removes expired cache rows
   - prompt hash change invalidates old investigation cache
3. Latest passing report:
   - `plans/reports/validation-260511-203629-agent-pipeline-live.md`
