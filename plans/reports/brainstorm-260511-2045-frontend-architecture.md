# Brainstorm Report: Frontend Architecture

**Date:** 2026-05-11 | **Status:** Finalized

---

## Problem

Backend Rust/Axum đã hoàn thành với SSE streaming endpoint (`GET /api/investigate`). Cần frontend để consume SSE, hiển thị investigation results real-time. Target: cả người dùng phổ thông lẫn nhà điều tra.

---

## Quyết định

| Item | Chọn | Lý do |
|------|------|-------|
| Framework | SvelteKit | Bundle 15KB (vs 85KB React), direct DOM = streaming mượt hơn, SSR/SEO built-in |
| Styling | Tailwind CSS | Utility-first, nhanh build UI, responsive, không cần component lib cho MVP |
| Layout | Google-style | Logo + search center, kết quả bên dưới. Đơn giản, quen thuộc |
| Scope | MVP 1 trang | Search box + streaming results. Không auth, không history |
| Domain | tracuuluadao.vn | Local dev trước, mua domain sau |

---

## Architecture

### Project Structure

```
frontend/
├── src/
│   ├── routes/
│   │   ├── +page.svelte          # Home: search + results
│   │   ├── +page.server.ts       # SSR meta tags cho SEO
│   │   └── +layout.svelte        # Root layout (fonts, global styles)
│   ├── lib/
│   │   ├── components/
│   │   │   ├── search-box.svelte
│   │   │   ├── phase-tracker.svelte     # Phase progress indicators
│   │   │   ├── source-card.svelte       # Per-source summary card
│   │   │   ├── detective-report.svelte  # Markdown rendered narrative
│   │   │   └── risk-badge.svelte        # Risk level badge
│   │   ├── stores/
│   │   │   └── investigation.ts         # SSE consumer + reactive state
│   │   └── utils/
│   │       ├── sse-client.ts            # EventSource wrapper
│   │       └── markdown.ts              # Markdown → HTML renderer
│   ├── app.css                          # Tailwind imports
│   └── app.html
├── static/
│   └── favicon.svg
├── svelte.config.js
├── tailwind.config.js
├── vite.config.ts
└── package.json
```

### SSE Consumer

```typescript
// lib/stores/investigation.ts
import { writable } from 'svelte/store';

export const investigation = writable({
  status: 'idle',       // idle | loading | streaming | complete | error
  phases: [],           // Phase progress tracking
  summaries: [],        // Per-source summaries
  urlAssessment: null,  // Selected URLs
  extractions: [],      // URL extractions
  detective: '',        // Streaming markdown text
  riskLevel: null,      // Final risk assessment
  confidence: null,
  duration: null,
});

export function startInvestigation(query: string, type: string) {
  const es = new EventSource(`/api/investigate?q=${encodeURIComponent(query)}&type=${type}`);
  
  es.addEventListener('phase_start', (e) => { /* update phases */ });
  es.addEventListener('source_status', (e) => { /* update source cards */ });
  es.addEventListener('progress', (e) => { /* update loading state */ });
  es.addEventListener('summary_result', (e) => { /* append summary */ });
  es.addEventListener('url_assessment', (e) => { /* set assessment */ });
  es.addEventListener('extraction_result', (e) => { /* append extraction */ });
  es.addEventListener('detective_stream', (e) => { /* append to detective text */ });
  es.addEventListener('complete', (e) => { /* set final results */ });
  es.addEventListener('error', (e) => { /* handle error */ });
}
```

### UI Components

#### 1. Search Box
- Input: placeholder "Nhập SĐT, STK ngân hàng, hoặc URL..."
- Auto-detect query type (phone/bank/url) via regex
- Button: "Tra cứu" with loading spinner
- Keyboard: Enter to submit

#### 2. Phase Tracker
- Horizontal steps: Thu thập → Phân tích → Đánh giá URL → Trích xuất → Tổng hợp
- Active step highlighted, completed steps checked
- Collapse sau khi hoàn thành

#### 3. Source Cards
- Per-source: icon + name + found count + summary text
- Appear one-by-one as summaries arrive
- Expandable for full details

#### 4. Detective Report
- Markdown rendered (streaming — text appears token-by-token)
- Sections: Bằng chứng → Đánh giá → Khuyến nghị
- Source links clickable

#### 5. Risk Badge
- Color-coded: 🔴 critical / 🟠 high / 🟡 medium / 🟢 low / ⚪ unknown
- Sticky at bottom hoặc top khi scroll
- Confidence percentage

### SEO

- SSR meta tags: title, description dựa trên query
- `robots.txt`: allow indexing
- Schema.org structured data cho search results
- Open Graph tags cho sharing

### Mobile Responsive

- Search box: full-width trên mobile
- Source cards: stack vertical
- Detective report: full-width, readable font size
- Phase tracker: horizontal scroll nếu nhỏ

---

## API Integration

### Proxy SSE (SvelteKit → Rust Backend)

```
Frontend (port 5173) → SvelteKit server → Proxy → Rust backend (port 3000)
```

Dùng SvelteKit server endpoint để proxy SSE, tránh CORS issues:

```typescript
// src/routes/api/investigate/+server.ts
export async function GET({ url }) {
  const backendUrl = `http://localhost:3000/api/investigate${url.search}`;
  const response = await fetch(backendUrl);
  return new Response(response.body, {
    headers: { 'Content-Type': 'text/event-stream' }
  });
}
```

---

## Dependencies

```json
{
  "devDependencies": {
    "@sveltejs/adapter-node": "^5",
    "@sveltejs/kit": "^2",
    "svelte": "^5",
    "tailwindcss": "^4",
    "typescript": "^5",
    "vite": "^6"
  },
  "dependencies": {
    "marked": "^15"
  }
}
```

Minimal deps: chỉ `marked` cho Markdown rendering. Tailwind 4 (native CSS, không cần PostCSS plugin).

---

## Phases

| # | Phase | Effort | Description |
|---|-------|--------|-------------|
| 1 | SvelteKit Bootstrap | S | Init project, Tailwind, layout, config |
| 2 | Search + SSE Consumer | M | Search box, EventSource, Svelte stores |
| 3 | Results UI | M | Phase tracker, source cards, risk badge |
| 4 | Detective Report | M | Markdown streaming render, styling |
| 5 | SEO + Polish | S | Meta tags, responsive fixes, loading states |

---

## Unresolved Questions

1. **Markdown renderer** — `marked` đủ hay cần `remark`/`rehype` cho custom rendering (highlight risk levels)?
2. **Dark mode** — MVP skip hay include? Tailwind dark mode rất dễ thêm.
3. **Rate limiting UI** — Nếu backend rate limit, frontend hiện gì?
4. **Share results** — URL có chứa query không? (e.g., `tracuuluadao.vn/?q=0926408013`)
