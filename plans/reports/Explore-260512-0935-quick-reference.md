# Frontend Quick Reference Guide

## 🎯 What is This?
Fraud investigation frontend for Vietnamese users to check phone numbers, bank account numbers, and URLs for scam risk using real-time SSE streaming.

**Tech Stack:** SvelteKit 2.57 + Svelte 5 + TypeScript 6 + Tailwind CSS 3 + Marked.js + DOMPurify

---

## 📁 File Paths (Absolute)

### Core Files
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/types.ts` — Type definitions
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/sse-client.ts` — SSE handler
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/routes/+page.svelte` — Main page
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/routes/+layout.svelte` — Layout + dark mode

### Component Files
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/components/search-box.svelte`
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/components/phase-tracker.svelte`
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/components/source-card.svelte`
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/components/url-assessment.svelte`
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/components/extraction-card.svelte`
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/components/detective-report.svelte`
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/components/risk-badge.svelte`

### Utilities
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/markdown.ts` — Markdown sanitizer
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/app.css` — Tailwind config
- `/home/khoa2807/working-sources/tracuuluadao/frontend/src/routes/+page.ts` — URL param loader

---

## 🧩 Component Cheat Sheet

```
SearchBox
├─ Input field with auto-detection
├─ Props: onSearch(query, type)
├─ Detects: phone (0\d{9,10}), bank (\d{8,20}), url (http* or domain)
└─ Size: 66 lines

PhaseTracker
├─ 5-phase progress bar with labels
├─ Props: phases[], progressMessage
├─ Phases: Thu thập → Phân tích → Đánh giá URL → Trích xuất → Tổng hợp
└─ Size: 53 lines

SourceCard
├─ Card per source with summary + facts
├─ Props: source (SourceStatusEvent), summary? (SummaryResultEvent)
├─ Icons: checkscam.vn🔍 chongluadao.vn🛡️ getguard.com📱 hadu.co🔎
└─ Size: 78 lines

UrlAssessment
├─ Blue info box listing selected URLs
├─ Props: assessment (UrlAssessmentEvent)
├─ Shows: priority badges, reasons, clickable links
└─ Size: 30 lines

ExtractionCard
├─ Card per extracted URL with entities & signals
├─ Props: extraction (ExtractionResultEvent)
├─ Shows: URL, summary, blue entity badges, red risk badges
└─ Size: 41 lines

DetectiveReport
├─ Scrollable markdown prose report
├─ Props: text (markdown), done (boolean)
├─ Features: auto-scroll, DOMPurify sanitization, animated cursor
└─ Size: 25 lines

RiskBadge
├─ Fixed bottom sheet with final score
├─ Props: completion (CompleteEvent)
├─ Shows: risk level (critical/high/medium/low/unknown), confidence%, duration
└─ Size: 49 lines
```

---

## 🔌 SSE Event Types (from backend)

```typescript
phase_start           → PhaseStartEvent {phase, label, total_sources?}
source_status         → SourceStatusEvent {source, status, found}
progress              → ProgressEvent {phase, message}
summary_result        → SummaryResultEvent {source, result{...}}
url_assessment        → UrlAssessmentEvent {selected, total, urls[]}
extraction_result     → ExtractionResultEvent {url, result{...}}
detective_stream      → DetectiveStreamEvent {chunk, done, replace}
complete              → CompleteEvent {risk_level, confidence, sources_analyzed, duration_ms}
investigation_error   → ErrorEvent {phase?, message, recoverable}
```

---

## 🎨 CSS Classes (from app.css)

```css
.risk-critical   /* Red 100/900 */
.risk-high       /* Orange 100/900 */
.risk-medium     /* Yellow 100/900 */
.risk-low        /* Green 100/900 */
.risk-unknown    /* Gray 100/900 */
.phase-pulse     /* Blue ring animation 1.5s */
```

---

## 🚀 Key Patterns

### Svelte 5 Runes
```typescript
let count = $state(0);              // Reactive state
let doubled = $derived(count * 2);  // Computed property
$effect(() => { /* runs when deps change */ });
```

### Dark Mode
```typescript
localStorage.getItem('theme') === 'dark' || 
!localStorage.getItem('theme') && matchMedia('(prefers-color-scheme: dark)').matches
```
Then: `document.documentElement.classList.toggle('dark', dark)`

### SSE Streaming
```typescript
createInvestigation(query, type, {
  onPhaseStart: (event) => { /* handle */ },
  onSourceStatus: (event) => { /* handle */ },
  // ... 7 more callbacks
  onComplete: (event) => { /* cleanup */ },
  onError: (event) => { /* retry or show error */ }
})
```

### Component Props
```typescript
let { onSearch }: { onSearch: (q: string, t: QueryType) => void } = $props();
```

---

## 📊 State Architecture (+page.svelte)

**12 state variables** + **1 derived** = entire app state

```typescript
// Input
currentQuery: string

// SSE events (accumulated)
phases: PhaseStartEvent[]
sources: SourceStatusEvent[]
summaries: SummaryResultEvent[]
urlAssessment: UrlAssessmentEvent | null
extractions: ExtractionResultEvent[]
detectiveText: string

// Final result
completion: CompleteEvent | null

// UI
status: 'idle' | 'loading' | 'streaming' | 'complete' | 'error'
error: string | null
progressMessage: string

// Connection
sseHandle: { close: () => void } | null

// Derived
sourceDisplay = sources.map(s => ({ 
  source: s, 
  summary: summaries.find(sm => sm.source === s.source) 
}))
```

---

## 🔄 User Flow

```
User types "0912345678"
        ↓
SearchBox detects: 'phone'
        ↓
Click "Tra cứu" button
        ↓
+page.handleSearch() resets state + opens SSE
        ↓
SSE events stream in → components render in real-time
        ↓
PhaseTracker shows progress (1→2→3→4→5)
SourceCard shows results as summaries arrive
UrlAssessment shows selected URLs
ExtractionCard shows details per URL
DetectiveReport accumulates markdown chunks
        ↓
CompleteEvent arrives
        ↓
RiskBadge appears at bottom with final assessment
SSE closes
```

---

## 🔒 Security Features

1. **Markdown Sanitization**
   ```typescript
   const html = marked.parse(md);
   DOMPurify.sanitize(html, { ADD_ATTR: ['target', 'rel', 'class'] });
   ```

2. **URL Encoding**
   ```typescript
   `/api/investigate?q=${encodeURIComponent(query)}&type=${type}`
   ```

3. **External Link Safety**
   ```html
   <a href={url} target="_blank" rel="noopener noreferrer">
   ```

4. **JSON Parsing Safety**
   ```typescript
   try { callbacks.onPhaseStart(JSON.parse(e.data)); } catch { /* skip */ }
   ```

---

## ⚠️ Known Issues / TODOs

1. SearchBox `loading` state not reset after search completes
2. AgentSummary.phone_mentions never displayed
3. AgentExtraction.related_numbers never displayed
4. No skeleton loaders during streaming
5. Progress messages don't timeout
6. Could add more accessibility labels
7. Very long URLs in extraction-card could break on mobile

---

## 📈 Code Metrics

| Metric | Value |
|--------|-------|
| Total Files | 14 |
| Total Lines | 811 |
| Largest File | +page.svelte (235 L) |
| Smallest File | +page.ts (8 L) |
| Components | 7 |
| Avg Component Size | 52 L |
| Type Definitions | 10 interfaces + 2 types |

---

## 🎯 Architecture in One Sentence

**Single-page SvelteKit app that streams SSE events from backend and renders them progressively across 7 reusable components managed by a single reactive state hub (+page.svelte) with dark mode support and responsive Tailwind CSS.**

