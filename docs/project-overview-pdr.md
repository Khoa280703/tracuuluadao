# Project Overview & PDR (Product Development Requirements)

> Last updated: 2026-05-24

## Project Purpose

Tra Cuu Lua Dao is a real-time scam investigation platform for Vietnamese users. Given a phone number, bank account, or URL, the system queries multiple anti-scam data sources, applies LLM-powered analysis across a multi-phase pipeline, and delivers a narrative detective report with a risk score through a Server-Sent Events (SSE) investigation stream.

The platform now also maintains a persistent knowledge base in PostgreSQL. Repeated investigations can reuse historical context, approved community reports, and detected links between related subjects.

## Problem Statement

Vietnamese users face increasing online fraud via phone scams, fake bank accounts, and phishing URLs. Existing anti-scam platforms are fragmented, requiring manual checks across multiple websites. A stateless investigation flow also loses useful context after each request. This platform consolidates data from multiple sources into a single investigation, then accumulates durable evidence over time.

## Target Users

- Vietnamese individuals checking suspicious phone numbers, bank accounts, or URLs
- Users who received suspicious messages with links or payment requests
- Community members submitting reports about scam behavior they experienced
- Internal moderators reviewing pending community reports

---

## Product Requirements

### Functional Requirements

| ID | Requirement | Status |
|----|-------------|--------|
| FR-1 | Accept phone number, bank account, or URL as input | Implemented |
| FR-2 | Query multiple anti-scam data sources in parallel | Implemented |
| FR-3 | LLM-powered source summarization (key facts, risk signals) | Implemented |
| FR-4 | AI-assisted URL selection for deep analysis | Implemented |
| FR-5 | Deep page extraction (entities, related numbers, risk signals) | Implemented |
| FR-6 | Synthesized detective report with risk level and confidence score | Implemented |
| FR-7 | Real-time SSE streaming of investigation progress | Implemented |
| FR-8 | Cache scraper results, LLM outputs, and full investigations | Implemented |
| FR-9 | Frontend: search with auto-detected query type and manual override | Implemented |
| FR-10 | Frontend: single-column narrative stream that replays phase and evidence events in arrival order | Implemented |
| FR-11 | Frontend: expandable evidence cards for source summaries and URL extractions | Implemented |
| FR-12 | Frontend: buffered detective conclusion shown after investigation completion, with markdown rendering and machine-footer stripping | Implemented |
| FR-13 | Frontend: final risk badge with normalized risk labels and clamped confidence display | Implemented |
| FR-14 | Frontend: dark mode with system preference detection | Implemented |
| FR-15 | Frontend: shareable URLs via query parameters | Implemented |
| FR-16 | Frontend: SEO meta tags (title, description, Open Graph) | Implemented |
| FR-17 | Frontend: XSS sanitization for user-facing content | Implemented |
| FR-18 | Fallback investigation when LLM is unavailable | Implemented |
| FR-19 | Hot-reload for agent prompt configuration | Implemented |
| FR-20 | Proxy rotation for search scrapers | Implemented |
| FR-21 | Frontend: preserve narrative continuity on recoverable SSE/backend failures by showing fallback conclusion in-place | Implemented |
| FR-22 | Persist investigated subjects, evidence, and investigation outcomes in PostgreSQL | Implemented |
| FR-23 | Load and expose historical context for repeated investigations | Implemented |
| FR-24 | Inject historical context into the detective agent when prior data exists | Implemented |
| FR-25 | Detect and persist subject links (phone, bank, URL) from extracted evidence | Implemented |
| FR-26 | Public subject APIs for history lookup and link network lookup | Implemented |
| FR-27 | Community report submission API with anonymous rate limiting and duplicate protection | Implemented |
| FR-28 | Admin moderation APIs to list, approve, and reject community reports | Implemented |
| FR-29 | Frontend: show historical context, linked subjects, and community report submission UI | Implemented |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR-1 | Investigation timeout | 180s max |
| NFR-2 | Per-scraper timeout | 10s max |
| NFR-3 | SSE failure handling | Recoverable backend errors preserve narrative context; terminal transport errors surface retry state |
| NFR-4 | Cache TTL (scrape results) | 24 hours |
| NFR-5 | Cache TTL (LLM analysis) | 1 hour |
| NFR-6 | Security (XSS prevention) | DOMPurify sanitization |
| NFR-7 | Accessibility | ARIA labels on interactive elements |
| NFR-8 | Responsive design | Mobile-first Tailwind CSS |
| NFR-9 | TypeScript type safety | Full typing with `svelte-check` |
| NFR-10 | Persistent knowledge base durability | Survives process restarts when `DATABASE_URL` is configured |
| NFR-11 | Community report abuse control | Max 5 reports per hashed IP per 24h, no duplicate report for same subject within 24h |
| NFR-12 | Risk recalculation consistency | Approved community reports update aggregate subject risk |

---

## System Components

### Backend (Rust)

- **Language**: Rust 2024 edition
- **Framework**: Axum 0.8 + Tokio
- **Database**: PostgreSQL (sqlx) for cache and persistent knowledge base
- **LLM**: Qwen 3.5 (JSON agents) + Qwen 3.6 (streaming detective)
- **Data Sources**: CheckScam, ChongLuaDao, TinNhiemMang, TrangTrang, Google, DuckDuckGo
- **Key Features**: SSE streaming, proxy pool, hot-reload agents, 3-tier cache, historical context enrichment, persistent evidence ingest, subject network graph, community report moderation APIs

### Frontend (SvelteKit)

- **Framework**: SvelteKit 2 + Svelte 5 (runes)
- **Styling**: Tailwind CSS 4
- **Build**: Vite 8 + adapter-node
- **Key Features**: narrative stream UI, SSE consumer, dark mode, markdown rendering, shareable URLs, historical context panel, linked-subject network panel, community report form
- **Security**: DOMPurify XSS sanitization, safe link rendering

---

## Data Sources

| Source | Type | Query Types |
|--------|------|-------------|
| CheckScam | Anti-scam database | phone, bank, url |
| TinNhiemMang | News aggregation | phone, bank, url |
| ChongLuaDao | Anti-scam database | phone |
| TrangTrang | News source | phone |
| Google | Search engine | phone, bank, url |
| DuckDuckGo | Search fallback | phone, bank, url |

Community reports are stored separately in the persistent knowledge base and only affect aggregate risk after admin approval.

---

## Investigation Pipeline

```
User Input (q, type)
       │
       ├─ Optional preload from persistent knowledge base
       │    └─ Historical context + linked-subject summary
       ▼
Phase 1: Data Collection ──────► Parallel scraper execution (6 sources)
       │
       ▼
Phase 2: Source Summarization ─► LLM agent summarizes each source
       │
       ▼
Phase 3: URL Selection ────────► LLM agent selects top URLs
       │
       ▼
Phase 4: URL Deep Analysis ────► Fetch + LLM extraction per URL
       │
       ▼
Phase 5: Detective Report ─────► Synthesized markdown + risk score
       │
       ├─ Report chunk stream buffered into Redis Streams when available
       ├─ Final result ingested into persistent knowledge base if quality score >= 0.3
       └─ Subject links recalculated from extracted mentions
       ▼
Process SSE + Report Replay SSE ─► Narrative timeline + buffered final report + fixed risk badge
```

Historical context is emitted early as a dedicated SSE event and is also passed into the detective prompt as `historical_context` when a known subject already exists.

---

## API Contract

### Investigation APIs

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| GET | `/api/investigate?q={query}&type={type}` | SSE investigation |
| GET | `/api/investigate/report?investigation_id={id}` | SSE buffered report replay |

### Subject & Knowledge Base APIs

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/subjects/{value}?type={type}` | Return subject history, recent investigations, approved reports, linked subjects, and risk signals |
| GET | `/api/subjects/{value}/network?type={type}&depth={1..3}` | Return recursive network graph rooted at the normalized subject |

### Community Report APIs

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/reports` | Submit an anonymous community report for a subject |
| GET | `/api/admin/reports?status=pending&limit=50&offset=0` | List pending or reviewed reports |
| POST | `/api/admin/reports/{id}/approve` | Approve report and recalculate subject risk |
| POST | `/api/admin/reports/{id}/reject` | Reject report and recalculate subject risk |

### Community Report Request Rules

`POST /api/reports` expects:

```json
{
  "value": "0926408013",
  "subject_type": "phone",
  "description": "Gọi mạo danh ngân hàng, yêu cầu chuyển tiền.",
  "category": "scam"
}
```

Validation rules:
- `subject_type` must be one of `phone`, `bank`, `url`
- `description` is required and capped at 2000 characters
- subject value is normalized before lookup (`phone` digits only, `bank` no whitespace, `url` lowercase without trailing slash)
- per-IP rate limit: 5 reports per 24h
- duplicate report for same subject from same IP is blocked for 24h

### Admin API Auth

- Header required: `x-admin-key`
- Value must match backend env `ADMIN_API_KEY`
- If `ADMIN_API_KEY` is absent, admin APIs remain disabled and return config errors

Frontend runtime note:
- The browser opens SSE against `${VITE_API_BASE_URL}/api/investigate?q={query}&type={type}` when `VITE_API_BASE_URL` is set, otherwise it falls back to same-origin `/api/investigate?...`.
- After `complete`, the browser replays the buffered report from `${VITE_API_BASE_URL}/api/investigate/report?...` or same-origin fallback.
- The frontend also calls `/api/subjects/{value}/network` and `/api/reports` through the configured API base URL.

---

## Configuration

### Backend Environment Variables (`.env.example`)

| Variable | Description | Required |
|----------|-------------|----------|
| `APP_HOST` | Bind address (default: `0.0.0.0`) | No |
| `APP_PORT` | Server port (default: `3067`) | No |
| `DATABASE_URL` | PostgreSQL connection string for cache and persistent knowledge base | Optional (disables cache and knowledge base) |
| `REDIS_URL` | Redis connection string for buffered report replay | Optional (falls back to direct SSE report) |
| `ADMIN_API_KEY` | Shared secret for admin moderation APIs | Optional (admin APIs disabled when missing) |
| `INVESTIGATION_REPORT_TTL_SECS` | TTL for buffered report stream in Redis | Optional |
| `PROXY_DIR` | Directory for proxy files | No |
| `AGENT_CONFIG_DIR` | Directory for agent TOML configs | No |
| `VITE_API_BASE_URL` | Frontend build-time API base URL for split web/API deployments | Optional |
| Qwen 3.5 endpoint variable | Qwen 3.5 API endpoint | Yes |
| Qwen 3.6 endpoint variable | Qwen 3.6 API endpoint | Yes |

### Frontend Build

| Command | Description |
|---------|-------------|
| `npm run dev` | Start dev server (Vite, port 5167) |
| `npm run build` | Production build |
| `npm run preview` | Preview production build |

---

## Success Metrics

| Metric | Target |
|--------|--------|
| Investigation completion rate | >90% |
| Cache hit rate | >50% for repeated queries |
| Average investigation time | <30s (non-cached) |
| Historical context hit rate | Increasing over time for repeat subjects |
| Community report moderation latency | <24h for pending reports |
| Subject network availability | Linked graph available after successful ingest for related subjects |

---

## Known Limitations

1. DuckDuckGo only activates when Google scraper fails.
2. Fallback detective report uses deterministic scoring when the model is unavailable.
3. Cache and knowledge base are both disabled when `DATABASE_URL` is not configured.
4. Subject history and network APIs only return data after at least one investigation or report has been persisted for that normalized subject.
5. Admin moderation currently uses a single shared header secret instead of scoped accounts.
6. Admin moderation still relies on a single shared `ADMIN_API_KEY` secret instead of scoped accounts or sessions.
7. Frontend depends on `/api/investigate` and `/api/subjects` being routed to the Rust backend outside SvelteKit route code.

---

## Future Considerations

- Admin dashboard for report moderation
- Public subject detail page backed by `/api/subjects/{value}`
- Ranking views on top of `subject_risk_overview`
- Rate limiting on `/api/investigate`
- Multi-language support (English interface)
- Mobile PWA support
