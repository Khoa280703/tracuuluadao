# Frontend Codebase Exploration Report
**Date:** 2026-05-12 | **Scope:** Very Thorough | **Stack:** SvelteKit 5 + TypeScript + Tailwind CSS

---

## 📊 Project Overview

**Frontend Framework:** SvelteKit 2.57.0 | **Svelte:** 5.55.2 | **TypeScript:** 6.0.2

**Total Codebase:** 811 lines across 14 files (components, lib utilities, routes, styles)

**Architecture:** Server-Sent Events (SSE) streaming with reactive state management, dark mode support, responsive Tailwind CSS

---

## 1️⃣ TypeScript Interfaces (`lib/types.ts`) — 36 lines

**File:** `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/types.ts`

Core type definitions for SSE events and application data:

```typescript
export type QueryType = 'phone' | 'bank' | 'url';

export interface AgentSummary {
  source: string;
  summary: string;
  key_facts: string[];
  phone_mentions: string[];
  risk_signals: string[];
}

export interface AgentExtraction {
  url: string;
  summary: string;
  entities: string[];
  risk_signals: string[];
  related_numbers: string[];
}

export interface SelectedUrl {
  url: string;
  reason: string;
  priority: number;
}

export type RiskLevel = 'critical' | 'high' | 'medium' | 'low' | 'unknown';

// SSE event payloads
export interface PhaseStartEvent { phase: number; label: string; total_sources?: number; }
export interface SourceStatusEvent { source: string; status: string; found: number; }
export interface ProgressEvent { phase: number; message: string; }
export interface SummaryResultEvent { source: string; result: AgentSummary; }
export interface UrlAssessmentEvent { selected: number; total: number; urls: SelectedUrl[]; }
export interface ExtractionResultEvent { url: string; result: AgentExtraction; }
export interface DetectiveStreamEvent { chunk: string; done: boolean; replace: boolean; }
export interface CompleteEvent { risk_level: RiskLevel; confidence: number; sources_analyzed: number; duration_ms: number; }
export interface ErrorEvent { phase?: number; message: string; recoverable: boolean; }
```

**Key Purpose:** Defines all types flowing through the SSE pipeline and UI state

---

## 2️⃣ SSE Client (`lib/sse-client.ts`) — 71 lines

**File:** `/home/khoa2807/working-sources/tracuuluadao/frontend/src/lib/sse-client.ts`

**Imports:** `EventSource`, types from `./types`

**Key Function:** `createInvestigation(query: string, type: QueryType, callbacks: SSECallbacks)`

```typescript
export interface SSECallbacks {
  onPhaseStart: (event: PhaseStartEvent) => void;
  onSourceStatus: (event: SourceStatusEvent) => void;
  onProgress: (event: ProgressEvent) => void;
  onSummaryResult: (event: SummaryResultEvent) => void;
  onUrlAssessment: (event: UrlAssessmentEvent) => void;
  onExtractionResult: (event: ExtractionResultEvent) => void;
  onDetectiveStream: (event: DetectiveStreamEvent) => void;
  onComplete: (event: CompleteEvent) => void;
  onError: (event: ErrorEvent) => void;
}
```

**Event Handling Pattern:**
- Opens `EventSource` to `/api/investigate?q=<query>&type=<type>`
- Registers 9 event listeners for different event types
- Safe JSON parsing with silent failures (try-catch per listener)
- Tracks completion state to prevent duplicate close calls
- Auto-closes on connection error with error callback
- Returns handle with `.close()` method for cleanup

---

## 3️⃣ Main Page (`routes/+page.svelte`) — 235 lines

**File:** `/home/khoa2807/working-sources/tracuuluadao/frontend/src/routes/+page.svelte`

**Imports:** 8 components, SSE client, Svelte transitions, types

**State Management (Svelte 5 Runes):**
```typescript
let status = $state<'idle' | 'loading' | 'streaming' | 'complete' | 'error'>('idle');
let phases = $state<PhaseStartEvent[]>([]);
let sources = $state<SourceStatusEvent[]>([]);
let summaries = $state<SummaryResultEvent[]>([]);
let urlAssessment = $state<UrlAssessmentEvent | null>(null);
let extractions = $state<ExtractionResultEvent[]>([]);
let detectiveText = $state('');
let completion = $state<CompleteEvent | null>(null);
let error = $state<string | null>(null);
let progressMessage = $state('');
let currentQuery = $state(initialQ);
let sseHandle = $state<{ close: () => void } | null>(null);
```

**Derived State:**
- `sourceDisplay` — maps sources to summaries for paired rendering

**Key Flow:**
1. **Search Trigger** (`handleSearch`):
   - Resets all state
   - Closes previous SSE connection
   - Updates URL params with `goto()`
   - Initiates new SSE connection with 9 callbacks

2. **SSE Callbacks:**
   - Accumulate events into arrays (phases, sources, summaries, extractions)
   - Update `detectiveText` with streaming chunks (replace or append)
   - Set status to 'streaming' on first phase
   - Close connection and set 'complete' on completion event
   - Handle errors with error callback

3. **Auto-Start:** If URL has `?q=...&type=...` on mount, auto-triggers search

4. **Rendering Logic:**
   - Shows empty state when `status === 'idle'`
   - Shows error box with retry button
   - Renders phase tracker, source cards, URL assessment, extraction cards, detective report
   - Shows risk badge when completion event received

**Meta Tags:** Dynamic title, OG properties, robots directive

---

## 4️⃣ Layout (`routes/+layout.svelte`) — 43 lines

**File:** `/home/khoa2807/working-sources/tracuuluadao/frontend/src/routes/+layout.svelte`

**Dark Mode Implementation:**
```typescript
let dark = $state(
  browser
    ? localStorage.getItem('theme') === 'dark'
      || (!localStorage.getItem('theme') && matchMedia('(prefers-color-scheme: dark)').matches)
    : false
);

$effect(() => {
  if (browser) {
    document.documentElement.classList.toggle('dark', dark);
    localStorage.setItem('theme', dark ? 'dark' : 'light');
  }
});
```

**Structure:**
- Fixed header with theme toggle button (sun/moon SVG)
- Main slot renders page content
- Background: `bg-white dark:bg-gray-950`
- Text: `text-gray-900 dark:text-gray-100`
- Z-index 50 for header (above results)

---

## 5️⃣ CSS (`app.css`) — 43 lines

**File:** `/home/khoa2807/working-sources/tracuuluadao/frontend/src/app.css`

**Key Styles:**

```css
@import "tailwindcss";
@plugin "@tailwindcss/typography";

/* Risk level color scheme */
.risk-critical { @apply bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-300; }
.risk-high { @apply bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-300; }
.risk-medium { @apply bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-300; }
.risk-low { @apply bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-300; }
.risk-unknown { @apply bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300; }

/* Phase pulse animation */
@keyframes pulse-ring {
  0% { transform: scale(0.95); box-shadow: 0 0 0 0 rgba(59, 130, 246, 0.5); }
  70% { box-shadow: 0 0 0 8px rgba(59, 130, 246, 0); }
  100% { transform: scale(0.95); box-shadow: 0 0 0 0 rgba(59, 130, 246, 0); }
}

.phase-pulse { animation: pulse-ring 1.5s ease-in-out infinite; }
```

**Typography:** Tailwind prose with dark mode invert support for markdown rendering

---

## 6️⃣ Svelte Components in `lib/components/`

### 6.1 search-box.svelte — 66 lines
**Purpose:** Search input with auto-detection

**Props:**
- `onSearch: (query: string, type: QueryType) => void`

**Features:**
- Auto-detects query type (phone: `/^0\d{9,10}$/`, bank: `/^\d{8,20}$/`, url: `http*` or domain)
- Disabled state during loading
- Type badge display (SĐT/STK/URL)
- Enter key and button submit
- Search icon left, type badge + submit button right
- Dark mode variants

**Reactive Type:** Uses `$derived(detectQueryType(query))`

---

### 6.2 phase-tracker.svelte — 53 lines
**Purpose:** 5-phase progress visualization

**Props:**
- `phases: PhaseStartEvent[]`
- `progressMessage: string`

**Features:**
- 5 phases: Thu thập → Phân tích → Đánh giá URL → Trích xuất → Tổng hợp
- Visual states: unstarted (gray), active (blue pulse), completed (green checkmark)
- Connecting lines between phases (green when completed, gray otherwise)
- Phase labels hidden on mobile (`sm:inline`)
- Animated pulse on current phase
- Progress message below (animate-pulse)

---

### 6.3 source-card.svelte — 78 lines
**Purpose:** Display search result from a single source

**Props:**
- `source: SourceStatusEvent` (source name, status, found count)
- `summary?: SummaryResultEvent` (optional summary data)

**Features:**
- Icon mapping: checkscam.vn→🔍, chongluadao.vn→🛡️, getguard.com→📱, hadu.co→🔎
- Spin loader while fetching (no summary yet)
- "Not found" label when found=0
- Green checkmark when summary received
- Shows source name + result count
- Summary text display
- Red risk signal badges
- Collapsible key facts with expand/collapse button
- Slide transition on mount

---

### 6.4 url-assessment.svelte — 30 lines
**Purpose:** Display selected URLs for deep investigation

**Props:**
- `assessment: UrlAssessmentEvent` (selected count, total count, URL list with priority/reason)

**Features:**
- Blue info box styling
- Shows "Chọn X/Y URL để điều tra sâu" header
- Lists URLs with priority badges (1-indexed)
- Each URL is clickable link (opens in new tab)
- Shows reason for selection if available
- Slide transition

---

### 6.5 extraction-card.svelte — 41 lines
**Purpose:** Display deep extraction result from a URL

**Props:**
- `extraction: ExtractionResultEvent` (url, result with summary/entities/risk_signals/related_numbers)

**Features:**
- Globe icon 🌐, linked URL heading
- Shows extraction summary
- Blue entity badges
- Red risk signal badges with ⚠ icon
- Handles long URLs with break-all
- Slide transition

---

### 6.6 detective-report.svelte — 25 lines
**Purpose:** Stream and display final analysis report

**Props:**
- `text: string` (markdown content)
- `done: boolean` (stream complete flag)

**Features:**
- Uses `sanitizeMarkdown()` utility to convert markdown → safe HTML
- Prose formatting with dark mode support
- Max height 600px with y-scroll
- Auto-scrolls to bottom while streaming
- Animated pulse cursor when not done
- Effect hook scrolls container to bottom on updates

---

### 6.7 risk-badge.svelte — 49 lines
**Purpose:** Final risk assessment badge (bottom sheet)

**Props:**
- `completion: CompleteEvent` (risk_level, confidence, sources_analyzed, duration_ms)

**Features:**
- Fixed bottom position, z-index 40 (below header)
- Risk labels: critical→🔴RỦI RO RẤT CAO, high→🟠RỦI RO CAO, medium→🟡, low→🟢, unknown→⚪
- Color classes from `app.css` (risk-critical/high/medium/low/unknown)
- Shows confidence%, source count, duration
- Slide transition from bottom
- Backdrop blur effect
- Accessibility: role="status" aria-live="polite"

---

## 7️⃣ Supporting Files

### markdown.ts — 20 lines
**Purpose:** Safe markdown rendering

```typescript
import { marked } from 'marked';
import DOMPurify from 'dompurify';

export function sanitizeMarkdown(md: string): string {
  // Uses marked.js for MD → HTML
  // DOMPurify.sanitize() removes XSS, allows target/rel/class attributes
  // Custom renderer forces external links to open in new tab
}
```

---

### +page.ts — 8 lines
**Purpose:** Load URL search params

```typescript
export const load: PageLoad = ({ url }) => {
  return {
    q: url.searchParams.get('q') ?? '',
    type: url.searchParams.get('type') ?? 'phone',
  };
};
```

---

### app.d.ts — 13 lines
Minimal namespace declaration (stub, no custom Locals/Error interfaces)

---

## 🔗 Component Connection Diagram

```
┌─────────────────────────────────────────────┐
│         +page.svelte (STATE HUB)           │
│  ┌─ SSE Event Listeners                     │
│  ├─ State: phases, sources, summaries, etc. │
│  └─ Manages SSE connection lifecycle        │
└─────────────────────────────────────────────┘
    │         │              │         │         │
    ▼         ▼              ▼         ▼         ▼
┌─────────┐┌──────────┐┌──────────┐┌─────────┐┌──────────┐
│Search   ││Phase     ││Source    ││URL      ││Extraction│
│Box      ││Tracker   ││Card      ││Assess   ││Card      │
└─────────┘└──────────┘└──────────┘└─────────┘└──────────┘
    ▲                                              │
    │                                              ▼
  User Input                                 ┌─────────────┐
                                             │Detective    │
                                             │Report       │
                                             └─────────────┘
                                                    │
                                                    ▼
                                             ┌─────────────┐
                                             │Risk Badge   │
                                             │(Bottom)     │
                                             └─────────────┘
```

---

## 📋 Data Flow: Query → SSE → UI

```
User enters "0912345678"
        ↓
  Search-box detects: 'phone'
        ↓
  +page.svelte:handleSearch() triggers
        ↓
  SSE createInvestigation('/api/investigate?q=...&type=phone')
        ↓
  Server sends events in sequence:
    • phase_start {phase: 1, label: "Thu thập"}
    • source_status {source: "checkscam.vn", found: 3}
    • summary_result {source: "checkscam.vn", result: {...}}
    • url_assessment {selected: 2, total: 5, urls: [...]}
    • extraction_result {url: "...", result: {...}}
    • detective_stream {chunk: "...", done: false}
    • detective_stream {chunk: "...", done: true}
    • complete {risk_level: "high", confidence: 92, ...}
        ↓
  UI renders components in real-time as state updates
        ↓
  Risk badge appears at bottom with final assessment
```

---

## 🎨 Dark Mode Implementation

**Storage:** `localStorage.theme` | **System Preference:** `prefers-color-scheme`

**Precedence:** Stored preference > System preference > Default (false)

**Toggle:** Button in fixed header with sun/moon icon SVG

**Application:** `document.documentElement.classList.toggle('dark', dark)` → triggers `:where(.dark, .dark *)` selectors in Tailwind

---

## 🔐 Security Notes

**SSE Client:**
- Silently catches JSON parsing errors (no console pollution)
- Validates all event data through TypeScript interfaces

**Markdown:**
- Uses DOMPurify to sanitize HTML
- Custom marked.js renderer prevents JavaScript: links
- External links forced to `target="_blank" rel="noopener noreferrer"`

**URL Handling:**
- Query params encoded with `encodeURIComponent()`
- Extraction URLs opened as new tabs with noopener

---

## 📱 Responsive Design

**Breakpoints:** Tailwind sm: (640px)

**Key Patterns:**
- Container max-w-2xl for results
- Phase labels hidden on mobile (`hidden sm:inline`)
- Flex layout with gap spacing
- Touch-friendly button sizes (py-4, px-5)
- Text sizes: sm/lg hierarchy

---

## ✅ Summary Table

| File | Lines | Purpose | Key Props |
|------|-------|---------|-----------|
| types.ts | 36 | Type definitions | QueryType, RiskLevel, 9 event interfaces |
| sse-client.ts | 71 | SSE connection | createInvestigation(query, type, callbacks) |
| +page.svelte | 235 | Main page hub | N/A (data from load + SSE) |
| +layout.svelte | 43 | Dark mode + header | N/A (context provider) |
| app.css | 43 | Tailwind + risk colors | .risk-* classes, phase-pulse animation |
| search-box.svelte | 66 | Query input | onSearch callback |
| phase-tracker.svelte | 53 | Progress bar | phases[], progressMessage |
| source-card.svelte | 78 | Source result | source, summary? |
| url-assessment.svelte | 30 | URL selection | assessment |
| extraction-card.svelte | 41 | URL extraction | extraction |
| detective-report.svelte | 25 | Markdown stream | text, done |
| risk-badge.svelte | 49 | Final score | completion |
| markdown.ts | 20 | MD sanitizer | sanitizeMarkdown(md) |
| +page.ts | 8 | URL loader | load(PageLoad) |

**Total: 811 lines across 14 files**

---

## ❓ Observations & Questions

1. **Loading State on Re-search:** The `loading` state in search-box is set but never reset when search completes. Should be reset when `status` changes to 'complete'.

2. **SSE Error Recovery:** Error callback has `recoverable: boolean` but +page doesn't distinguish handling. Recoverable errors might benefit from auto-retry logic.

3. **Phone Mentions Unused:** `AgentSummary.phone_mentions[]` is defined but never rendered in any component.

4. **Related Numbers Unused:** `AgentExtraction.related_numbers[]` is defined but never rendered in extraction-card.

5. **Missing Loading State Indicator:** While streaming, there's no skeleton loader or placeholder for cards that haven't arrived yet.

6. **Prose Styling:** Detective report uses Tailwind prose, but body text size/line-height not explicitly set—relies on prose defaults.

7. **Phase Message Duration:** Progress message shows but doesn't timeout—could clutter UI if many messages queued.

8. **URL Truncation:** Extraction card uses `break-all` for URLs; on mobile, very long URLs could break layout.

