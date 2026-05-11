# Phase 5: Pipeline Orchestration

## Priority: Critical | Effort: L | Status: completed

## Overview

Core investigation pipeline: coordinates scrapers → LLM agents → streaming output. This is the "brain" of the system.

## Current Reality

- `src/pipeline/investigation.rs` implements the full 5-phase flow, emits phase/progress/result events, and streams detective output.
- Summarizer and extractor phases run concurrently; URL assessor has fallback selection when model output is empty/invalid.
- Cancellation checks, global 60s timeout wrapper, detective degrade fallback, and full-investigation/analysis cache hooks are wired through the flow.
- Summarizer failures now degrade into compact raw-source summaries instead of silently dropping evidence.
- End-to-end live validation is recorded in `plans/reports/validation-260511-203629-agent-pipeline-live.md`.

## Requirements

- 5-phase pipeline execution (scrape → summarize → assess → extract → detective)
- Parallel execution within phases (multiple sources/URLs simultaneously)
- Streaming results as each phase produces output
- Graceful degradation if any phase fails
- Pipeline state management for progress tracking

## Architecture

```
src/pipeline/
├── mod.rs              # Pipeline struct, public API
├── state.rs            # PipelineState tracking (phases, progress)
├── investigation.rs    # Full investigation flow
└── url_fetcher.rs      # Fetch URLs selected by URL Assessor
```

### Pipeline Flow

```rust
pub struct Investigation {
    pub query: String,
    pub query_type: QueryType,  // Phone, BankAccount, Url
}

pub async fn run_investigation(
    investigation: Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    proxy_pool: Option<Arc<ProxyPool>>,
    cache: Option<Arc<CacheService>>,
    cancel_token: CancellationToken,
    tx: mpsc::Sender<InvestigationEvent>,
) -> Result<InvestigationResult>
```

### Phase Execution

```
Phase 1: scrape_all(query, proxy_pool, cache)
    → emit: phase_start, source_status per source
    → collect: Vec<ScrapedResult>

Phase 2: for each scraped source with content:
    → emit: progress event ("Đang phân tích {source}...")
    → call summarizer agent (4B, NON-STREAMING, parallel)
    → JSON repair if needed (retry once)
    → emit: summary_result event (complete JSON when done)
    → collect: Vec<AgentSummary>

Phase 3: if google/ddg results exist:
    → call url_assessor agent (4B, NON-STREAMING, single call)
    → emit: url_assessment event
    → collect: Vec<SelectedUrl>

Phase 4: for each selected URL:
    → fetch page content (HTTP first, skip JS-only for v1)
    → emit: progress event ("Đang phân tích {url}...")
    → call extractor agent (4B, NON-STREAMING, parallel)
    → JSON repair if needed
    → emit: extraction_result event (complete JSON when done)
    → collect: Vec<AgentExtraction>

Phase 5: combine all summaries + extractions:
    → call detective agent (27B, streaming)
    → emit: detective_stream chunks (`replace: false`)
    → cached/fallback full-body paths emit `replace: true`
    → parse final `risk_level` + `confidence`
    → emit: complete event
```

## Implementation Steps

### 1. State Tracking (`state.rs`)
```rust
pub struct PipelineState {
    pub phase: u8,
    pub total_sources: usize,
    pub completed_sources: usize,
    pub summaries: Vec<AgentSummary>,
    pub selected_urls: Vec<SelectedUrl>,
    pub extractions: Vec<AgentExtraction>,
    pub start_time: Instant,
}
```

### 2. Investigation Flow (`investigation.rs`)

1. **Phase 1 — Scraping:**
   - `tokio::join!` all scrapers
   - Emit `phase_start` + `source_status` events
   - Filter sources with actual content

2. **Phase 2 — Summarization:**
   - For each source with content, spawn summarizer call
   - Use `futures::stream::FuturesUnordered` for parallel execution
   - Emit `summary_result` event as each summary completes (not wait for all)
   - Parse JSON response → `AgentSummary`

3. **Phase 3 — URL Assessment:**
   - Build input: search results + summaries context (for relevance)
   - Single LLM call to url-assessor
   - Parse JSON → list of URLs with priority
   - If fails → fallback: select top 5 URLs by snippet relevance

4. **Phase 4 — Deep Extraction:**
   - Fetch each selected URL content
   - Truncate to ~3000 chars (4B context limit consideration)
   - Call extractor agent per URL (parallel)
   - Emit `extraction_result` event as each extraction completes

5. **Phase 5 — Detective Synthesis:**
   - Build detective prompt: all summaries + extractions
   - Stream 27B response token-by-token to frontend
   - Require detective output footer:
     `RISK_LEVEL: <critical|high|medium|low|unknown>`
     `CONFIDENCE: <0.0-1.0>`
   - Parse `risk_level` + `confidence` from footer lines after stream completes

### 3. URL Fetcher (`url_fetcher.rs`)
- HTTP GET with Chrome UA (rquest TLS impersonation)
- Extract main content (strip nav, footer, ads)
- If page requires JS → mark for Lightpanda fallback (future)
- Truncate to max chars for LLM context
- Timeout: 5s per URL

### 4. Agent Response Parsing
```rust
pub struct AgentSummary {
    pub source: String,
    pub summary: String,
    pub key_facts: Vec<String>,
    pub phone_mentions: Vec<String>,
    pub risk_signals: Vec<String>,
}

pub struct AgentExtraction {
    pub url: String,
    pub summary: String,
    pub entities: Vec<String>,
    pub risk_signals: Vec<String>,
    pub related_numbers: Vec<String>,
}
```

### 5. Fallback & Error Handling
- Summarizer fails → include raw content in detective prompt
- URL Assessor fails → visit top 5 URLs by default
- Extractor fails for URL → skip, log warning
- Detective fails → return raw summaries to user with "LLM unavailable" note
- Overall timeout: 60s for entire pipeline
- Every long-running step checks `cancel_token.is_cancelled()` before spawning new work
- Wrap scraper, fetch, and LLM futures in `tokio::select!` so client disconnect stops upstream work quickly

## Success Criteria

- [x] End-to-end orchestration code exists across all planned phases
- [x] Events are emitted in pipeline order to the SSE channel
- [x] Phase 5 detective stream path is implemented
- [x] Graceful degradation exists for URL selection fallback and per-step recoverable errors
- [x] Full pipeline verified end-to-end with live phone query in this pass
- [x] Pipeline completes within 60s total
- [x] Graceful degradation: detective/LLM hard failure returns raw data instead of terminal error

## Risk Assessment

- **4B model concurrent overload** — Limit parallel summarizer calls to 4 (vLLM queue)
- **Detective context overflow** — If many sources, truncate older/lower-priority summaries
- **Streaming backpressure** — If frontend slow, use bounded channel with drop-oldest policy
