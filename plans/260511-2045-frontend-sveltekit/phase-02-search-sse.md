# Phase 2: Search + SSE Consumer

## Priority: Critical | Effort: M | Status: complete

## Overview

Search box with query type auto-detection, SSE client consuming backend events, reactive state management with Svelte 5 runes.

## Implementation Steps

### 1. SSE Client (`lib/sse-client.ts`)

```typescript
export function createInvestigation(query: string, type: QueryType) {
  const url = `/api/investigate?q=${encodeURIComponent(query)}&type=${type}`;
  const es = new EventSource(url);

  // Register typed event handlers
  es.addEventListener('phase_start', handler);
  es.addEventListener('source_status', handler);
  // ... all 9 event types

  return { close: () => es.close() };
}
```

Key behaviors:
- Parse each SSE event JSON → dispatch to callback
- `onerror` → close EventSource, set error state
- Return close handle for cleanup on navigation/new search
- Auto-close on `complete` event

### 2. Page State (`+page.svelte`)

Use Svelte 5 runes for reactive state:

```svelte
<script lang="ts">
  let status = $state<'idle'|'loading'|'streaming'|'complete'|'error'>('idle');
  let phases = $state<PhaseStartEvent[]>([]);
  let sources = $state<SourceStatusEvent[]>([]);
  let summaries = $state<SummaryResultEvent[]>([]);
  let urlAssessment = $state<UrlAssessmentEvent|null>(null);
  let extractions = $state<ExtractionResultEvent[]>([]);
  let detectiveText = $state('');
  let completion = $state<CompleteEvent|null>(null);
  let error = $state<string|null>(null);
  let currentPhase = $derived(phases.at(-1)?.phase ?? 0);
</script>
```

### 3. URL State Sync (`+page.ts`)

```typescript
// +page.ts — load URL params for shareable links
export function load({ url }) {
  return {
    q: url.searchParams.get('q') ?? '',
    type: url.searchParams.get('type') ?? 'phone',
  };
}
```

On search submit → `goto(`?q=${query}&type=${type}`)` → triggers load → starts SSE.

### 4. Search Box Component (`lib/components/search-box.svelte`)

- Input with placeholder "Nhập SĐT, STK ngân hàng, hoặc URL..."
- Auto-detect query type via regex:
  - Phone: `/^0\d{9,10}$/` → `phone`
  - Bank: `/^\d{8,20}$/` (non-phone digit string) → `bank`
  - URL: starts with `http` or contains `.` → `url`
- Submit button with loading spinner
- Disabled during active investigation
- Enter key submits

### 5. Query Type Badge

Small pill next to input showing detected type: "SĐT" / "STK" / "URL"
Updates reactively as user types.

## Related Files

- Create: `frontend/src/lib/sse-client.ts`
- Create: `frontend/src/lib/components/search-box.svelte`
- Create: `frontend/src/routes/+page.svelte`
- Create: `frontend/src/routes/+page.ts`

## Success Criteria

- [x] Search submits → SSE connection opens → events received
- [x] URL updates with query params on search
- [x] Sharing URL `?q=0926408013` auto-triggers search on load
- [x] Query type auto-detected correctly (phone/bank/url)
- [x] Previous investigation cancelled when new search starts
- [x] Error state shown when backend unreachable
