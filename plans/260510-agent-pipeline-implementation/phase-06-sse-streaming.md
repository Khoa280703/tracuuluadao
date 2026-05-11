# Phase 6: SSE Streaming Endpoint

## Priority: High | Effort: M | Status: completed

## Overview

Axum HTTP endpoint that accepts a query, runs the investigation pipeline, and streams results via Server-Sent Events (SSE).

## Current Reality

- Router, SSE handler, event serialization, permissive CORS, keep-alive, and cancellation-on-disconnect are implemented.
- Verified 2026-05-11: `/health` responds with `ok`, `proxies_loaded`, `uptime_seconds`, and `cache_enabled`.
- Verified 2026-05-11: app also boots without `DATABASE_URL`; `/health` then reports `cache_enabled:false`.
- Verified 2026-05-11: `/api/investigate` accepts `type=phone|bank|url`; `curl` against `type=phone` and `type=bank` both return SSE `phase_start` immediately.
- `detective_stream` now includes `replace` so clients can swap in cached/fallback full-body markdown instead of appending.
- `AppState` now carries optional cache service and process start time for health reporting.

## Requirements

- `GET /api/investigate?q={query}&type={phone|bank|url}` → SSE stream
- Events follow protocol defined in brainstorm report
- CORS support for frontend on different port
- Client disconnect → CancellationToken drops → pipeline stops (resource cleanup)
- ~~Request deduplication~~ (DEFERRED to v2 — YAGNI for initial launch)

## Architecture

```
src/api/
├── mod.rs              # Router setup
├── investigate.rs      # SSE investigation endpoint
└── health.rs           # Health check
```

### SSE Event Protocol

```
event: phase_start
data: {"phase": 1, "label": "Thu thập dữ liệu", "total_sources": 5}

event: source_status
data: {"source": "checkscam.vn", "status": "done", "found": 3}

event: progress
data: {"phase": 2, "message": "Đang phân tích checkscam report 1..."}

event: summary_result
data: {"source": "checkscam_report_1", "result": {"summary": "...", "key_facts": [...], "risk_signals": [...]}}

event: url_assessment
data: {"selected": 4, "total": 10, "urls": [...]}

event: progress
data: {"phase": 4, "message": "Đang phân tích https://example.com..."}

event: extraction_result
data: {"url": "...", "result": {"summary": "...", "entities": [...], "risk_signals": [...]}}

event: detective_stream
data: {"chunk": "## Kết quả điều tra...", "done": false, "replace": false}

event: complete
data: {"risk_level": "high", "confidence": 0.85, "sources_analyzed": 8, "duration_ms": 28400}

event: error
data: {"phase": 2, "message": "LLM timeout", "recoverable": true}
```

## Implementation Steps

### 1. Router (`mod.rs`)
```rust
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/investigate", get(investigate::handler))
        .route("/health", get(health::handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
```

### 2. SSE Handler (`investigate.rs`)
```rust
struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub async fn handler(
    Query(params): Query<InvestigateParams>,
    State(state): State<AppState>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let (tx, rx) = mpsc::channel::<InvestigationEvent>(128);
    let cancel_token = CancellationToken::new();
    let cancel_for_task = cancel_token.clone();
    let app_state = Arc::new(state);
    
    tokio::spawn(async move {
        let investigation = Investigation {
            query: params.q,
            query_type: params.r#type.parse().unwrap_or(QueryType::Phone),
        };
        if let Err(e) = run_investigation(
            investigation,
            app_state.registry.clone(),
            app_state.llm.clone(),
            Some(app_state.proxy_pool.clone()),
            app_state.cache.clone(),
            cancel_for_task.clone(),
            tx.clone(),
        ).await {
            let _ = tx.send(InvestigationEvent::Error {
                phase: None,
                message: e.to_string(),
                recoverable: false,
            }).await;
        }
    });

    let stream = async_stream::stream! {
        let _guard = CancelOnDrop(cancel_token);
        let mut rx = ReceiverStream::new(rx);
        while let Some(event) = futures::StreamExt::next(&mut rx).await {
            yield Ok(Event::default()
                .event(event.event_name())
                .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string())));
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
```

### 3. Event Types
```rust
#[derive(Serialize)]
#[serde(tag = "type")]
pub enum InvestigationEvent {
    PhaseStart { phase: u8, label: String, total_sources: Option<usize> },
    SourceStatus { source: String, status: String, found: usize },
    Progress { phase: u8, message: String },
    SummaryResult { source: String, result: AgentSummary },
    UrlAssessment { selected: usize, total: usize, urls: Vec<SelectedUrl> },
    ExtractionResult { url: String, result: AgentExtraction },
    DetectiveStream { chunk: String, done: bool, replace: bool },
    Complete { risk_level: String, confidence: f32, sources_analyzed: usize, duration_ms: u64 },
    Error { phase: Option<u8>, message: String, recoverable: bool },
}
```

### 4. AppState
```rust
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<AgentRegistry>,
    pub llm: Arc<LlmClient>,
    pub proxy_pool: Arc<ProxyPool>,
    pub cache: Option<Arc<CacheService>>,
    pub started_at: Instant,
}
```

## Success Criteria

- [x] SSE endpoint and event protocol are implemented in Axum
- [x] Client disconnect cancellation and permissive CORS are implemented
- [x] Health endpoint returns 200
- [x] `curl .../api/investigate?q=0926408013&type=phone` live stream verified in this pass
- [x] `type=bank|url` support is implemented
- [x] Health endpoint returns 200 with uptime info

## Risk Assessment

- **Connection drops mid-stream** — Use `tokio::select!` with cancellation token
- **Slow consumers** — Bounded channel (64 events), drop if full + log warning
- **Concurrent investigations** — Each request spawns own pipeline, vLLM handles queuing
