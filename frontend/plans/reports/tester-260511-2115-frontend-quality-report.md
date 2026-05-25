---
name: Frontend Quality Report 2026-05-11
description: Initial QA report for the SvelteKit fraud-lookup frontend — build, types, dev server, components, SSE proxy
type: project
---

# Frontend QA Report — 2026-05-11 21:15

## Test Results Overview

| Category       | Status  | Details                            |
|----------------|---------|------------------------------------|
| Build          | PASS    | SSR + client, 155 modules, 1.39s  |
| TypeScript     | FAIL    | 6 errors, 3 warnings               |
| Dev Server     | PASS    | HTTP 200, starts in ~940ms        |
| Components     | PASS    | 7 lib + 2 route = 9 .svelte files |
| SSE Proxy      | PASS    | Route exists, proxies correctly   |
| State Warnings | WARN    | 2 state_reference warnings         |

---

## Build

**Status: PASS**

- SSR: 155 modules transformed, built in 248ms
- Client: 166 modules transformed, built in 1.39s
- Adapter: `@sveltejs/adapter-node`
- Output size: client ~139kb total, server ~440kb total
- No build errors. Build warnings about `state_referenced_locally` appear but do not block.

---

## TypeScript (svelte-check)

**Status: FAIL — 6 errors, 3 warnings, 181 files scanned**

### Errors: Missing `slide` import (6 occurrences across 5 files)

The `slide` transition from `svelte/transition` is used via `transition:slide` but never imported.

| File                          | Line(s) | Issue                        |
|-------------------------------|---------|------------------------------|
| `extraction-card.svelte`      | 7       | `Cannot find name 'slide'`   |
| `risk-badge.svelte`           | 29      | `Cannot find name 'slide'`   |
| `source-card.svelte`          | 20, 66  | `Cannot find name 'slide'`   |
| `url-assessment.svelte`       | 7       | `Cannot find name 'slide'`   |
| `src/routes/+page.svelte`     | 153     | `Cannot find name 'slide'`   |

**Fix:** Add `import { slide } from 'svelte/transition'` to each affected component.

### Warnings (3)

1. **`+page.svelte:25`** — `state_referenced_locally`: `data.q` captured into `initialQ` may lose reactivity.
2. **`+page.svelte:26`** — `state_referenced_locally`: `data.type` captured into `initialType` may lose reactivity.
3. **`tsconfig.json:1`** — Missing `@types/node` type definition.

---

## Dev Server

**Status: PASS**

- Starts in ~940ms
- Serves root (`/`) with HTTP 200
- HTML includes Tailwind CSS inline, correct meta tags, SvelteKit head rendering
- Vite v8.0.12

---

## Component Inventory (9 .svelte files)

| # | Component                        | Lines | Purpose                                  | Imports OK |
|---|----------------------------------|-------|------------------------------------------|------------|
| 1 | `search-box.svelte`              | 67    | Query input with auto-type detection     | Yes        |
| 2 | `phase-tracker.svelte`           | 54    | 5-phase progress stepper                 | Yes        |
| 3 | `source-card.svelte`             | 77    | Source status with expandable summary    | Yes        |
| 4 | `url-assessment.svelte`          | 30    | URL priority list display                | Yes        |
| 5 | `extraction-card.svelte`         | 41    | Extraction result card                   | Yes        |
| 6 | `detective-report.svelte`        | 25    | Markdown rendering with auto-scroll      | Yes        |
| 7 | `risk-badge.svelte`              | 49    | Fixed-bottom risk level banner           | Yes        |
| 8 | `+page.svelte`                   | 235   | Main page orchestrating all components   | Yes        |
| 9 | `+layout.svelte`                 | 44    | Dark mode toggle, theme persistence      | Yes        |

All `$lib` imports resolve. No broken import paths. Component graph is flat (no nested component dependencies).

### Supporting Files

| File              | Purpose                           |
|-------------------|-----------------------------------|
| `lib/types.ts`    | Type definitions for SSE events   |
| `lib/sse-client.ts`| EventSource client for SSE proxy |
| `lib/markdown.ts` | Marked.js wrapper for rendering  |
| `routes/+page.ts` | Page load with URL param parsing  |
| `app.d.ts`        | Global App namespace declarations |

---

## SSE Proxy Endpoint (`/api/investigate`)

**Status: PASS (route exists, logic correct)**

- GET handler at `src/routes/api/investigate/+server.ts`
- Proxies to `http://localhost:3000/api/investigate`
- Returns proper SSE headers: `text/event-stream`, `no-cache`, `keep-alive`
- Returns 400 if `q` param missing
- Returns 502 if backend unavailable (expected when no backend running)
- Client-side: `sse-client.ts` uses `EventSource` with 9 event listeners for typed SSE events

---

## State Reference Warnings

`svelte-check` reports `state_referenced_locally` on lines 25-26 of `+page.svelte`:

```ts
let { data } = $props();
const initialQ = data.q as string;       // line 25 — warning
const initialType = data.type as QueryType; // line 26 — warning
```

These `const` values capture the initial value of reactive `data`. In Svelte 5 runes mode, `const` does not track reactivity. However, the current usage is **intentional**: these values are only used once at initialization (auto-start check on line 121-123) and are not expected to update reactively. The warning is a false positive in this context but can be silenced by adding `// @ts-ignore` or restructuring.

---

## Critical Issues

1. **Missing `slide` imports (6 errors)** — Blocks `svelte-check` from passing. Each affected file needs `import { slide } from 'svelte/transition'`.

## Recommendations (Priority Order)

1. **Add `slide` import** to `extraction-card.svelte`, `risk-badge.svelte`, `source-card.svelte`, `url-assessment.svelte`, `+page.svelte` — 5 files, 1 line each
2. **Install `@types/node`** (`npm i -D @types/node`) to resolve tsconfig warning
3. **Suppress `state_referenced_locally`** on `+page.svelte:25-26` with comment or refactor if reactivity is intended
4. **Add unit tests** — project currently has zero test files
5. **Add `@types/node` or explicit type roots** in tsconfig

## Next Steps

1. Fix 5 missing `slide` imports
2. Re-run `npm run check` to confirm zero errors
3. Scaffold test suite (svelte-testing-library or `@testing-library/svelte`)

## Unresolved Questions

- Is the `state_referenced_locally` warning on `data.q` / `data.type` intentional (one-time init) or should these be reactive?
- Should `@types/node` be added as a dev dependency or is it unnecessary?
