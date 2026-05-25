# Phase 1: Narrative Components

## Overview
- **Priority:** P1
- **Effort:** 2h
- **Status:** completed
- **Description:** Shipped 3 new components for the narrative stream UI. Minor hardening landed inside component scope too.

## Context Links
- [Brainstorm](../../plans/reports/brainstorm-260511-2224-narrative-ux-redesign.md)
- [Narrative line](../../frontend/src/lib/components/narrative-line.svelte)
- [Evidence card](../../frontend/src/lib/components/evidence-card.svelte)
- [Narrative conclusion](../../frontend/src/lib/components/narrative-conclusion.svelte)

## Key Insights
- source-card (78L) and extraction-card (41L) share similar patterns: icon + summary + expandable details + risk tags
- evidence-card unifies both into one component with `variant` prop
- detective-report became `narrative-conclusion`, with machine footer stripped before render so backend footer does not leak into UI
- narrative-line stayed pure presentation: icon + text + optional typing animation
- evidence-card added safe URL parsing so malformed extraction URLs fall back to plain text instead of broken links

## Requirements

### Functional
- narrative-line renders a single text line with optional typing animation
- evidence-card renders expandable card for both summary and extraction results
- narrative-conclusion streams markdown with auto-scroll (same as current detective-report)

### Non-functional
- Each component under 100 lines
- Svelte 5 runes only ($state, $props, $derived)
- Tailwind CSS 4 classes
- Dark mode support

## Architecture

### NarrativeLine type (added to types.ts in Phase 2)

```ts
type NarrativeLineType = 'text' | 'evidence-summary' | 'evidence-extraction';

interface NarrativeLine {
  id: number;
  type: NarrativeLineType;
  text?: string;           // for 'text' type
  icon?: string;           // emoji prefix
  summary?: SummaryResultEvent;     // for 'evidence-summary'
  extraction?: ExtractionResultEvent; // for 'evidence-extraction'
  animate?: boolean;       // typing animation on appear
}
```

## Related Code Files

### Create
- `frontend/src/lib/components/narrative-line.svelte` (~50L)
- `frontend/src/lib/components/evidence-card.svelte` (~90L)
- `frontend/src/lib/components/narrative-conclusion.svelte` (~30L)

### Delete (in Phase 2 when page is rewritten)
- `frontend/src/lib/components/phase-tracker.svelte`
- `frontend/src/lib/components/source-card.svelte`
- `frontend/src/lib/components/url-assessment.svelte`
- `frontend/src/lib/components/extraction-card.svelte`

## Implementation Steps

### Step 1: Create `narrative-line.svelte` (~50L)

Simple presentational component. Renders one narrative text line.

```svelte
<!-- pseudocode -->
<script lang="ts">
  let { text, icon, animate = false }: Props = $props();
</script>

<div class="flex items-start gap-2 py-1.5" transition:slide>
  {#if icon}<span>{icon}</span>{/if}
  <p class={animate ? 'typing-animate' : ''}>{text}</p>
</div>
```

Props:
- `text: string` — the Vietnamese narrative text
- `icon?: string` — emoji prefix (e.g., "🔍", "⚠️")
- `animate?: boolean` — apply fade-in typing effect (default false)

### Step 2: Create `evidence-card.svelte` (~90L)

Unified expandable card for both summary_result and extraction_result. Uses discriminated variant prop.

```svelte
<!-- pseudocode -->
<script lang="ts">
  import type { SummaryResultEvent, ExtractionResultEvent } from '$lib/types';
  
  type Props = {
    variant: 'summary';
    data: SummaryResultEvent;
  } | {
    variant: 'extraction';
    data: ExtractionResultEvent;
  };
  
  let { variant, data }: Props = $props();
  let expanded = $state(false);
  
  // Derive display fields based on variant
  let title = $derived(variant === 'summary' ? data.source : data.url);
  let summary = $derived(variant === 'summary' ? data.result.summary : data.result.summary);
  let riskSignals = $derived(variant === 'summary' ? data.result.risk_signals : data.result.risk_signals);
  let icon = $derived(variant === 'summary' ? sourceIcon(data.source) : '🌐');
</script>

<!-- Collapsed: icon + title + 1-line summary + expand button -->
<div class="rounded-xl border ... p-3 my-2" transition:slide>
  <div class="flex items-start gap-2">
    <span>{icon}</span>
    <div class="flex-1 min-w-0">
      <span class="font-semibold">{title}</span>
      <p class="text-sm text-gray-600 truncate">{summary}</p>
    </div>
    <button onclick={() => expanded = !expanded}>
      {expanded ? '▲ Thu gọn' : '▼ Chi tiết'}
    </button>
  </div>
  
  <!-- Expanded: key_facts/entities + risk_signals tags -->
  {#if expanded}
    <div transition:slide>
      <!-- risk signal tags -->
      <!-- key_facts (summary) or entities (extraction) list -->
    </div>
  {/if}
</div>
```

Key details:
- Source icon map reused from current source-card.svelte (checkscam -> "🔍", etc.)
- Risk signals as red tags, entities as blue tags (same style as current)
- `slide` transition on expand/collapse

### Step 3: Create `narrative-conclusion.svelte` (~30L)

Rename of detective-report.svelte with minor changes:
- Remove wrapping card border (parent handles layout now)
- Keep markdown streaming + auto-scroll + cursor
- Keep `sanitizeMarkdown` dependency

```svelte
<!-- nearly identical to detective-report.svelte -->
<script lang="ts">
  import { sanitizeMarkdown } from '$lib/markdown';
  let { text, done }: { text: string; done: boolean } = $props();
  let html = $derived(text ? sanitizeMarkdown(text) : '');
  // auto-scroll effect same as current
</script>

<div bind:this={container} class="prose dark:prose-invert max-w-none">
  {@html html}
  {#if !done}
    <span class="animate-pulse text-gray-400">▋</span>
  {/if}
</div>
```

Changes from detective-report:
- Remove `max-h-[600px] overflow-y-auto` — parent scroll handles this
- Remove `p-2` padding — parent handles spacing
- File name: `narrative-conclusion.svelte`

## Todo List

- [x] Create narrative-line.svelte with text + icon + animate props
- [x] Create evidence-card.svelte with summary/extraction variant
- [x] Create narrative-conclusion.svelte (copy + modify detective-report)
- [x] Add safe malformed-URL fallback in evidence-card render path
- [x] Strip machine footer text before narrative conclusion render
- [x] Verify all components use Svelte 5 runes, no legacy stores
- [x] Verify dark mode works on all 3 components
- [x] Each file under 100 lines

## Success Criteria

- [x] narrative-line renders text with optional icon and fade-in animation
- [x] evidence-card renders collapsed view with 1-line summary
- [x] evidence-card expands to show key_facts/entities + risk tags
- [x] evidence-card works for both summary and extraction variants
- [x] narrative-conclusion streams markdown with typing cursor and strips machine footer before render
- [x] All components under 100 lines, Svelte 5 runes only

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Props type union may confuse TS | Low | Use explicit variant discriminator |
| evidence-card exceeding 100L | Medium | Extract sourceIcon map to shared util if needed |

## Next Steps

- Phase 2 consumes these components in +page.svelte rewrite
- Delete old components (phase-tracker, source-card, url-assessment, extraction-card) in Phase 2

## Verification

- `narrative-line.svelte` 13L, `evidence-card.svelte` 82L, `narrative-conclusion.svelte` 20L
- All 3 components use `$props`/`$state`/`$derived`; no legacy store pattern introduced
- Dark-mode classes present on all shipped components
