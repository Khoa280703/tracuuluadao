---
title: "Narrative UX Redesign"
description: "Replace card/stage UI with single-scroll narrative stream for investigator feel"
status: completed
priority: P1
effort: 6h
branch: main
tags: [frontend, ux, svelte5, narrative]
created: 2026-05-12
---

# Narrative UX Redesign

## Goal

Replace dashboard-style card/stage UI with a single-scroll narrative stream. User should feel like "someone is investigating for me" — SSE events map to Vietnamese narrative lines, evidence cards appear inline, 27B detective conclusion streams naturally.

## Current State

- Delivered. Narrative stream shipped across 11 active frontend/config files, 5 legacy components removed.
- Dashboard layout replaced by single-scroll investigation timeline with inline evidence + streamed conclusion.
- SSE pipeline stayed on same 9 event callbacks. No backend contract change.
- Small truthful scope drift landed too: `search-box.svelte` query-type hydration hardened, `risk-badge.svelte` risk parsing normalized, malformed extraction URLs render safe, machine footer stripped from narrative conclusion, recoverable SSE fallback no longer cuts the narrative.

## Phase Overview

| Phase | Description | Effort | Status |
|-------|-------------|--------|--------|
| [Phase 1](phase-01-narrative-components.md) | New components: narrative-line, evidence-card | 2h | completed |
| [Phase 2](phase-02-page-rewrite.md) | Rewrite +page.svelte + types update | 3h | completed |
| [Phase 3](phase-03-prompt-and-polish.md) | Detective prompt rewrite + CSS polish | 1h | completed |

## Files Impact

### Create
- `frontend/src/lib/components/narrative-line.svelte`
- `frontend/src/lib/components/evidence-card.svelte`
- `frontend/src/lib/components/narrative-conclusion.svelte` (rename from detective-report)
- `frontend/src/lib/narrative-stream-copy.ts`

### Major Modify
- `frontend/src/routes/+page.svelte` — full rewrite
- `frontend/src/lib/types.ts` — add NarrativeLine type
- `frontend/src/app.css` — typing animation, remove phase-pulse
- `config/agents/detective/prompt.md` — narrative tone
- `frontend/src/lib/components/search-box.svelte` — stronger URL/type hydration from route params
- `frontend/src/lib/components/risk-badge.svelte` — normalize parsed risk level before render

### Delete
- `frontend/src/lib/components/phase-tracker.svelte`
- `frontend/src/lib/components/source-card.svelte`
- `frontend/src/lib/components/url-assessment.svelte`
- `frontend/src/lib/components/extraction-card.svelte`
- `frontend/src/lib/components/detective-report.svelte`

### No Change
- `frontend/src/lib/sse-client.ts`
- `frontend/src/routes/+layout.svelte`
- `frontend/src/routes/+page.ts`

## Key Decisions

1. **No backend SSE changes** — all narrative mapping is client-side
2. **Typing animation via CSS** — no JS interval, use `@keyframes` on opacity
3. **Evidence cards reuse existing type interfaces** — AgentSummary, AgentExtraction unchanged
4. **NarrativeLine discriminated union** — shipped as `type: 'text' | 'evidence-summary' | 'evidence-extraction'`
5. **Auto-scroll** — window scroll + anchor, pauses if user scrolled up
6. **Keep `+page.svelte` cohesive** — file ended at 217L, slightly above target, to keep SSE flow, recoverable-error handling, and scroll logic in one place

## Dependencies

- Phase 2 depends on Phase 1 (components must exist)
- Phase 3 independent (prompt + CSS can run parallel with Phase 2)

## Risks

- Template narrative repetition mitigated by `narrative-stream-copy.ts`
- Evidence cards still render in arrival order; this matched desired "live investigation" feel
- Runtime regressions around fallback/error/footer/URL rendering resolved in shipped implementation

## Verification

- `npm --prefix frontend run check` — PASS, `svelte-check` found 0 errors and 0 warnings
- `npm --prefix frontend run build` — PASS
- Confirmed legacy components deleted: `phase-tracker`, `source-card`, `url-assessment`, `extraction-card`, `detective-report`
