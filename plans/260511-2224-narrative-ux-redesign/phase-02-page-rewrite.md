# Phase 2: Page Rewrite + Types Update

## Overview
- **Priority:** P1
- **Effort:** 3h
- **Status:** completed
- **Depends on:** Phase 1 (components must exist)
- **Description:** Rewrote `+page.svelte` from dashboard layout to narrative stream, updated `types.ts`, removed legacy components, and hardened adjacent UI pieces that risked regression.

## Context Links
- [Current +page.svelte](../../frontend/src/routes/+page.svelte) (217L) — shipped narrative page
- [Current types.ts](../../frontend/src/lib/types.ts) — add NarrativeLine type
- [Phase 1 components](phase-01-narrative-components.md)

## Key Insights
- Narrative stream did become the primary render model, but real implementation kept a few extra guards: `errorRecoverable`, `currentType`, `streamAnchor`, `routeKey`, `userScrolledUp`
- Narrative copy was extracted into `frontend/src/lib/narrative-stream-copy.ts` instead of keeping every sentence inline in `+page.svelte`
- Recoverable SSE errors stay visible without breaking the narrative stream or hiding the conclusion
- URL query hydration needed hardening in `search-box.svelte` so route params do not drift from manual type overrides
- `risk-badge.svelte` now normalizes parser output before render so stray punctuation does not break the final badge

## Requirements

### Functional
- SSE events map to NarrativeLine entries in a single array
- Narrative lines render sequentially in a scrollable container
- Evidence cards appear inline within the narrative flow
- Detective conclusion streams below narrative lines
- Risk badge appears at bottom after completion
- Search box stays at top, with stronger route hydration behavior than originally planned

### Non-functional
- `+page.svelte` stays close to the target size; shipped at 217L to keep SSE flow, retry/error copy, and scroll handling cohesive
- Remove all imports of deleted components
- Preserve URL query param sync (goto)
- Preserve auto-start from URL params

## Architecture

### State Model (shipped)

```
Primary state:
  status
  narrativeLines[]
  detectiveText
  completion
  error

Support state kept for UX stability:
  errorRecoverable
  currentQuery
  currentType
  streamAnchor
  userScrolledUp
  routeKey
  sseHandle
```

`progressMessage` was removed. Narrative lines replaced phase/status cards, but not every support flag disappeared.

### NarrativeLine Type (in types.ts)

```ts
export type NarrativeLineType = 'text' | 'evidence-summary' | 'evidence-extraction';

export interface NarrativeLine {
  id: number;
  type: NarrativeLineType;
  text?: string;
  icon?: string;
  summary?: SummaryResultEvent;
  extraction?: ExtractionResultEvent;
}
```

### SSE → Narrative Mapping (in +page.svelte)

```ts
let lineCounter = 0;
function addLine(line: Omit<NarrativeLine, 'id'>): void {
  narrativeLines = [...narrativeLines, { ...line, id: ++lineCounter }];
}

// SSE callbacks:
onPhaseStart: (e) => {
  if (e.phase === 1) addLine({ type: 'text', icon: '🔍', text: `Đang tiến hành phân tích ${currentQuery}...` });
  else if (e.phase === 3) addLine({ type: 'text', icon: '📋', text: 'Đang chọn các nguồn đáng tin cậy để điều tra sâu hơn...' });
  else if (e.phase === 4) addLine({ type: 'text', icon: '🌐', text: 'Đang truy vết sâu trên internet...' });
  else if (e.phase === 5) addLine({ type: 'text', icon: '🕵️', text: 'Đã thu thập đủ dữ liệu, đang tổng hợp kết luận...' });
},

onSourceStatus: (e) => {
  if (e.status === 'error') return; // skip silently
  if (e.found > 0) {
    addLine({ type: 'text', icon: '🔍', text: `Đã tìm thấy ${e.found} kết quả trên ${e.source}, tiếp tục truy vết...` });
  } else {
    addLine({ type: 'text', icon: '—', text: `Không tìm thấy thông tin trên ${e.source}.` });
  }
},

onSummaryResult: (e) => {
  addLine({ type: 'evidence-summary', summary: e });
},

onUrlAssessment: (e) => {
  addLine({ type: 'text', icon: '📌', text: `Đã xác định ${e.selected} URL cần truy vết chi tiết.` });
},

onExtractionResult: (e) => {
  addLine({ type: 'evidence-extraction', extraction: e });
},
```

Actual shipped mapping also routes:
- `onProgress` into narrative text lines with phase-aware icon copy
- recoverable `onError` into warning narrative without flipping the whole page into terminal error mode
- `onComplete` into final timeline line before showing `risk-badge`

### Template Layout (pseudocode)

```svelte
<div class="min-h-screen flex flex-col">
  <!-- Header + Search (same as current) -->
  
  <!-- Narrative Container -->
  {#if status !== 'idle'}
    <div class="w-full max-w-2xl mx-auto px-4" bind:this={narrativeContainer}>
      {#each narrativeLines as line (line.id)}
        {#if line.type === 'text'}
          <NarrativeLine text={line.text} icon={line.icon} />
        {:else if line.type === 'evidence-summary'}
          <EvidenceCard variant="summary" data={line.summary} />
        {:else if line.type === 'evidence-extraction'}
          <EvidenceCard variant="extraction" data={line.extraction} />
        {/if}
      {/each}
      
      {#if detectiveText}
        <div class="mt-4 pl-2 border-l-2 border-blue-400">
          <NarrativeConclusion text={detectiveText} done={status === 'complete'} />
        </div>
      {/if}
    </div>
  {/if}

  <!-- Risk Badge (same as current) -->
  {#if completion}
    <RiskBadge {completion} />
  {/if}
</div>
```

### Auto-scroll Logic

```ts
let narrativeContainer: HTMLDivElement | null = null;
let userScrolledUp = $state(false);

function handleScroll() {
  if (!narrativeContainer) return;
  const { scrollTop, scrollHeight, clientHeight } = narrativeContainer;
  userScrolledUp = scrollHeight - scrollTop - clientHeight > 100;
}

$effect(() => {
  // trigger on narrativeLines.length or detectiveText change
  if (narrativeContainer && !userScrolledUp) {
    narrativeContainer.scrollIntoView({ block: 'end', behavior: 'smooth' });
  }
});
```

Note: Use `window.scrollTo` or `scrollIntoView` on the last element since the page itself scrolls (no inner scroll container). Simpler approach: scroll window to bottom unless user scrolled up.

## Related Code Files

### Modify
- `frontend/src/lib/types.ts` — add NarrativeLine + NarrativeLineType exports
- `frontend/src/routes/+page.svelte` — full rewrite (shipped at 217L)
- `frontend/src/lib/components/search-box.svelte` — hydrate query/type from URL without stomping manual override
- `frontend/src/lib/components/risk-badge.svelte` — normalize backend footer values before badge render

### Delete
- `frontend/src/lib/components/phase-tracker.svelte`
- `frontend/src/lib/components/source-card.svelte`
- `frontend/src/lib/components/url-assessment.svelte`
- `frontend/src/lib/components/extraction-card.svelte`
- `frontend/src/lib/components/detective-report.svelte` (replaced by narrative-conclusion)

## Implementation Steps

### Step 1: Update types.ts

Add `NarrativeLineType` and `NarrativeLine` interface. Keep all existing types unchanged.

### Step 2: Rewrite +page.svelte

1. Update imports: remove old components, add narrative-line, evidence-card, narrative-conclusion
2. Replace dashboard-centric state with `narrativeLines` primary flow plus extra stability flags for scroll/error/url hydration
3. Add `addLine()` helper function
4. Rewrite SSE callbacks to use narrative mapping (see Architecture above)
5. Keep `handleSearch()` reset logic — clear `narrativeLines` array
6. Keep URL sync (`goto`), auto-start logic
7. Rewrite template: hero + search (keep) → narrative container (new) → risk badge (keep)
8. Add auto-scroll effect with `streamAnchor`
9. Keep error state UI, but make recoverable banner non-blocking for narrative flow
10. Keep idle/empty state UI

### Step 3: Delete old components

Remove 5 files:
- phase-tracker.svelte
- source-card.svelte
- url-assessment.svelte
- extraction-card.svelte
- detective-report.svelte

### Step 4: Verify

- `npm run check` — TypeScript compilation passes
- `npm run dev` — page renders, SSE events produce narrative lines
- Test with a real query to verify narrative flow

## Todo List

- [x] Add NarrativeLine type to types.ts
- [x] Rewrite +page.svelte imports
- [x] Replace dashboard render model with `narrativeLines` array as the primary stream
- [x] Implement addLine helper
- [x] Rewrite all 9 SSE callbacks with narrative mapping
- [x] Extract narrative copy to `frontend/src/lib/narrative-stream-copy.ts`
- [x] Rewrite template to narrative scroll layout
- [x] Add auto-scroll with user-scrolled-up detection
- [x] Keep error/idle/empty states
- [x] Make recoverable error banner preserve downstream narrative/conclusion visibility
- [x] Harden URL query type hydration in `search-box.svelte`
- [x] Normalize risk-level parsing in `risk-badge.svelte`
- [x] Delete 5 old component files
- [x] TypeScript check passes
- [x] Production build passes

## Success Criteria

- [x] `+page.svelte` kept cohesive at 217L; slight size overrun accepted to avoid premature extraction of coupled SSE/scroll/error logic
- [x] SSE events render as sequential narrative lines
- [x] Evidence cards appear inline with expandable details
- [x] Detective conclusion streams with cursor below narrative
- [x] Risk badge appears at bottom after complete
- [x] Auto-scroll works, pauses when user scrolls up
- [x] URL query params still sync with stronger type hydration behavior
- [x] No TypeScript errors
- [x] No references to deleted components remain

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Page exceeds 200L | Medium | Extract narrative mapping to `narrative-mapper.ts` utility |
| Auto-scroll jank on mobile | Low | Use `requestAnimationFrame` for scroll, test on mobile viewport |
| Evidence card type narrowing | Low | Discriminated union on variant prop handles this |

## Security Considerations
- No new user input surface added; `search-box` only tightened route/type hydration
- No new API calls — SSE client unchanged
- Markdown sanitization still via `sanitizeMarkdown` in narrative-conclusion

## Verification

- `npm --prefix frontend run check` — PASS, 0 errors, 0 warnings
- `npm --prefix frontend run build` — PASS
- Confirmed deleted: `phase-tracker`, `source-card`, `url-assessment`, `extraction-card`, `detective-report`
- Runtime hardening captured from shipped implementation: recoverable SSE fallback preserved, malformed extraction URLs render safe, footer machine text no longer leaks, error banner no longer covers narrative, URL query type hydration stronger
