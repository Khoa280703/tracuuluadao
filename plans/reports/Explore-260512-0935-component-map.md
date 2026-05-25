# Frontend Component Map & Architecture Reference

## 📂 File Structure

```
frontend/src/
├── app.css                          # Tailwind + custom styles (43 lines)
├── app.d.ts                         # Type declarations (13 lines)
├── lib/
│   ├── types.ts                     # Core TypeScript interfaces (36 lines)
│   ├── sse-client.ts                # SSE connection handler (71 lines)
│   ├── markdown.ts                  # Markdown sanitizer (20 lines)
│   └── components/
│       ├── search-box.svelte        # Input + auto-detect (66 lines)
│       ├── phase-tracker.svelte     # 5-phase progress (53 lines)
│       ├── source-card.svelte       # Source result card (78 lines)
│       ├── url-assessment.svelte    # URL selection box (30 lines)
│       ├── extraction-card.svelte   # URL extraction result (41 lines)
│       ├── detective-report.svelte  # Markdown report viewer (25 lines)
│       └── risk-badge.svelte        # Final score badge (49 lines)
└── routes/
    ├── +layout.svelte               # Dark mode + header (43 lines)
    ├── +page.svelte                 # Main page state hub (235 lines)
    └── +page.ts                     # URL param loader (8 lines)
```

## 🔄 Prop Flow Diagram

```
+layout.svelte
    ├─ dark: boolean (state)
    └─ layout wrapper (header + main)
        │
        └─ +page.svelte (child: children() slot)
            ├─ data prop (from +page.ts)
            │   ├─ q: string
            │   └─ type: QueryType
            │
            ├─ SearchBox
            │   └─ onSearch(query, type) → handleSearch()
            │
            ├─ PhaseTracker
            │   ├─ phases[]
            │   └─ progressMessage
            │
            ├─ SourceCard (repeated)
            │   ├─ source (SourceStatusEvent)
            │   └─ summary? (SummaryResultEvent)
            │
            ├─ UrlAssessment
            │   └─ assessment (UrlAssessmentEvent)
            │
            ├─ ExtractionCard (repeated)
            │   └─ extraction (ExtractionResultEvent)
            │
            ├─ DetectiveReport
            │   ├─ text: string (markdown)
            │   └─ done: boolean
            │
            └─ RiskBadge
                └─ completion (CompleteEvent)
```

## 🎯 Component Responsibilities

### Presentation Layer (7 Components)

| Component | Renders | Updates From | Size |
|-----------|---------|--------------|------|
| **SearchBox** | Input field + type badge | User typing, button click | 66 L |
| **PhaseTracker** | 5-step progress bar + message | Phases array, progressMessage | 53 L |
| **SourceCard** | Source name, icons, summary, facts | Sources + summaries arrays | 78 L |
| **UrlAssessment** | URL list with priorities | urlAssessment event | 30 L |
| **ExtractionCard** | URL with entities & signals | Extractions array | 41 L |
| **DetectiveReport** | Scrollable markdown prose | detectiveText state | 25 L |
| **RiskBadge** | Final score + metadata | Completion event | 49 L |

### Utility Layer (2 Files)

| Utility | Exports | Used By |
|---------|---------|---------|
| **types.ts** | 10 interfaces + 2 types | All components + sse-client |
| **sse-client.ts** | `createInvestigation()` + SSECallbacks | +page.svelte |
| **markdown.ts** | `sanitizeMarkdown()` | detective-report |

### Framework Layer (2 Files)

| Layer | Provides | Size |
|-------|----------|------|
| **+layout.svelte** | Dark mode toggle, header, CSS | 43 L |
| **app.css** | Tailwind + .risk-* classes + animations | 43 L |

---

## 📡 SSE Event Sequence

```
User triggers search
        ↓
sse-client opens EventSource
        ↓
Server sends stream...

Event Order:
┌─────────────────────────────────────────┐
│ 1. phase_start                          │ → PhaseTracker starts rendering
│    {phase: 1, label: "Thu thập"}        │   phases[] = [event]
├─────────────────────────────────────────┤
│ 2. source_status (multiple)             │ → SourceCard created per source
│    {source: "checkscam.vn", found: 3}   │   sources[] = [event1, event2, ...]
├─────────────────────────────────────────┤
│ 3. summary_result (matches source)      │ → SourceCard updates with summary
│    {source: "checkscam.vn", result: {}} │   summaries[] = [event]
├─────────────────────────────────────────┤
│ 4. url_assessment (single)              │ → UrlAssessment renders URLs
│    {selected: 2, total: 5, urls: [...]} │   urlAssessment = event
├─────────────────────────────────────────┤
│ 5. extraction_result (per selected URL) │ → ExtractionCard per URL
│    {url: "...", result: {...}}          │   extractions[] = [event1, ...]
├─────────────────────────────────────────┤
│ 6. detective_stream (chunks)            │ → DetectiveReport accumulates text
│    {chunk: "...", done: false}          │   detectiveText += chunk
├─────────────────────────────────────────┤
│ 7. complete (final)                     │ → RiskBadge appears, SSE closes
│    {risk_level: "high", ...}            │   completion = event
└─────────────────────────────────────────┘
```

---

## 🎨 Styling Architecture

### Tailwind Configuration
- **Base:** System fonts, dark mode via `.dark` class
- **Components:** Risk level badges (5 variants), phase-pulse animation
- **Typography:** Prose classes for markdown rendering

### Color Scheme by Risk Level
```
critical → 🔴 Red-100 bg + Red-800 text (dark: Red-900/30 bg + Red-300 text)
high     → 🟠 Orange (similar pattern)
medium   → 🟡 Yellow
low      → 🟢 Green
unknown  → ⚪ Gray
```

### Key Animations
- `phase-pulse`: 1.5s ring expansion on active phase (blue)
- `animate-spin`: Loading spinners
- `animate-pulse`: Progress message blink, detective report cursor
- `slide`: Component entrance transitions (Svelte transition)

---

## 🔗 Dependency Graph

```
Types (types.ts)
    ↑
    ├── sse-client.ts
    │       ↑
    │       └── +page.svelte
    │               ├── All 7 components
    │               ├── markdown.ts
    │               └── +layout.svelte
    │                   └── app.css
```

**Linear Dependency Path:**
```
app.css 
  ← +layout.svelte 
    ← +page.svelte 
      ← sse-client.ts ← types.ts
      ← 7 components ← markdown.ts
```

---

## 📊 State Management Detail

### +page.svelte State (12 variables)
```typescript
// Query & display
let currentQuery: string                    // User input + query history
let status: 'idle' | 'loading' | ...        // Page state machine
let error: string | null                    // Error message display

// SSE event accumulation
let phases: PhaseStartEvent[]               // All phases received
let sources: SourceStatusEvent[]            // All source statuses
let summaries: SummaryResultEvent[]         // All summaries (matches sources)
let urlAssessment: UrlAssessmentEvent | null   // Single URL selection
let extractions: ExtractionResultEvent[]   // All extracted URLs
let detectiveText: string                   // Markdown stream (append/replace)
let completion: CompleteEvent | null        // Final result

// UI state
let progressMessage: string                 // Current phase message
let sseHandle: { close: () => void } | null // SSE connection handle
```

### Derived State (1 variable)
```typescript
let sourceDisplay = $derived(              // Pairs sources with their summaries
  sources.map((s) => ({
    source: s,
    summary: summaries.find((sm) => sm.source === s.source),
  }))
);
```

---

## 🔐 Data Validation

### TypeScript Type Safety
- All 10 event interfaces defined in `types.ts`
- SSE callbacks typed via `SSECallbacks` interface
- Component props typed with `{ prop: Type }`

### Runtime Validation
- JSON parsing wrapped in try-catch (silent failures)
- Markdown sanitized with `DOMPurify.sanitize()`
- Query params encoded with `encodeURIComponent()`
- URLs opened with `rel="noopener noreferrer"`

---

## 📱 Responsive Breakpoints

### Mobile First (320px+)
- Full-width container
- Stacked layout
- Text sizes: text-lg for inputs, text-sm for details

### Tablet+ (640px, sm:)
- Phase labels shown (`hidden sm:inline`)
- Wider gaps between phase steps

---

## 🎯 Key Pattern: Svelte 5 Runes

### State Rune
```typescript
let variable = $state(initialValue);
```
Used for: status, phases, sources, summaries, error, etc.

### Derived Rune
```typescript
let computed = $derived(expression);
```
Used for: sourceDisplay, detectedType, icon mapping

### Effect Rune
```typescript
$effect(() => {
  // runs when dependencies change
});
```
Used for: dark mode toggle, detective report auto-scroll

---

## ❌ Known Gaps

1. **search-box.svelte:** `loading` state never reset after search completes
2. **types.ts:** `phone_mentions[]` and `related_numbers[]` never rendered
3. **+page.svelte:** No skeleton loaders while streaming
4. **detective-report:** No timeout on progress messages
5. **All components:** No accessibility labels beyond role="status"

---

## ✅ Strengths

1. **Type Safety:** Comprehensive interfaces, zero `any` types
2. **Component Isolation:** Each component has single responsibility
3. **Reactive:** Uses Svelte 5 runes for clean state/derived logic
4. **Dark Mode:** Proper theme persistence + system preference detection
5. **SSE Handling:** Robust error handling, safe JSON parsing
6. **Accessibility:** ARIA labels, semantic HTML, keyboard support
7. **Performance:** No unnecessary re-renders (derived state + runes)

