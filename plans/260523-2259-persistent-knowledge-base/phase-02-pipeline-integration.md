# Phase 2: Pipeline Integration (Ingest + Enrich)

## Context
- [Phase 1](./phase-01-schema-and-knowledge-base-service.md) — KnowledgeBase service phải hoàn thành trước
- Pipeline: `src/pipeline/investigation.rs` — `run_investigation_inner()` orchestrates 5 phases
- Detective prompt: `config/agents/detective/prompt.md`

## Overview
- **Priority:** P1
- **Status:** Complete
- **Progress:** 100%
- **Effort:** 6h
- Hook KnowledgeBase vào pipeline: pre-query historical data trước khi chạy pipeline, post-ingest kết quả sau khi hoàn thành (chỉ khi `quality_score >= 0.3`). Feed historical context vào detective agent.

## Key Insights
- Pipeline hiện tại: scrape → summarize → assess URLs → extract → detective → cache
- Thêm 2 điểm hook: **PRE** (trước scrape) và **POST** (sau cache)
- Pre-query KHÔNG block pipeline nếu KB unavailable
- Post-ingest chạy async (spawn) — không delay response về user
- Detective agent cần thêm section "Dữ liệu lịch sử" trong user prompt
- InvestigationEvent cần thêm variant `HistoricalContext` để frontend hiển thị

## Requirements

### Functional
- Pre-query: lookup subject in KB → load history (investigation count, risk history, approved reports, linked subjects)
- Enrich detective: inject historical context vào detective agent prompt
- Post-ingest: upsert subject + insert investigation + insert evidence (summaries + extractions) + recalculate risk
- Ingest gate: quality_score >= 0.3 (thay vì risk >= medium — vì investigation risk=low nhưng quality cao vẫn có giá trị xác nhận "không phải scam")
- Quality score computed at ingest: `sources_success_ratio × 0.4 + confidence × 0.3 + evidence_richness × 0.2 + recency × 0.1`
- SSE event: emit `historical_context` event nếu subject đã có trong KB
- Frontend: hiển thị historical badge

### Non-functional
- Pre-query latency < 50ms (single indexed query)
- Post-ingest không block SSE response
- Graceful: KB failure = skip silently, pipeline continues

## Architecture

```
run_investigation_inner()
  │
  ├─ [NEW] PRE-QUERY: kb.get_subject_by_value(query, query_type)
  │   └─ Nếu có: kb.get_subject_history(subject_id)
  │   └─ Emit InvestigationEvent::HistoricalContext { ... }
  │
  ├─ Phase 1: Scrape (unchanged)
  ├─ Phase 2: Summarize (unchanged)
  ├─ Phase 3: URL Assessment (unchanged)
  ├─ Phase 4: Extract URLs (unchanged)
  ├─ Phase 5: Detective Report
  │   └─ [MODIFIED] build_detective_input() includes historical context
  │
  ├─ Cache (unchanged)
  │
  └─ [NEW] POST-INGEST: tokio::spawn async
      ├─ Gate: quality_score >= 0.3?
      ├─ kb.upsert_subject()
      ├─ kb.insert_investigation()
      ├─ kb.insert_evidence_batch() — summaries + extractions
      └─ kb.recalculate_risk()
```

## Related Code Files

### Files to Modify
- `src/pipeline/investigation.rs` — add pre-query, modify detective input, add post-ingest
- `src/pipeline/state.rs` — add `HistoricalContext` event variant + `SubjectHistory` re-export
- `config/agents/detective/prompt.md` — add historical data section template

### Files NOT Modified
- `src/cache/mod.rs` — giữ nguyên
- `src/scrapers/*` — giữ nguyên
- `src/api/investigate.rs` — SSE stream tự emit event mới (no change needed)

## Implementation Steps

### 1. Add HistoricalContext event to `src/pipeline/state.rs`
```rust
// Thêm vào InvestigationEvent enum
HistoricalContext {
    investigation_count: i32,
    last_risk_level: String,
    approved_reports: i32,
    linked_subjects_count: i32,
    first_seen: String,  // ISO date
},
```
- Add `event_name()` match: `"historical_context"`

### 2. Pass KnowledgeBase into `run_investigation()`
- Add param: `knowledge_base: Option<Arc<KnowledgeBase>>`
- Thread through `run_investigation_inner()`
- Update call site in `src/api/investigate.rs`: pass `app_state.knowledge_base.clone()`

### 3. Implement pre-query in `run_investigation_inner()`
After cache check, before Phase 1:
```rust
let subject_history = if let Some(kb) = knowledge_base.as_ref() {
    match kb.get_subject_by_value(&investigation.query, investigation.query_type.as_str()).await {
        Ok(Some(subject)) => {
            match kb.get_subject_history(subject.id, 10).await {
                Ok(history) => {
                    tx.send(InvestigationEvent::HistoricalContext {
                        investigation_count: history.subject.investigation_count,
                        last_risk_level: history.recent_investigations.first()
                            .map(|i| i.risk_level.clone())
                            .unwrap_or_default(),
                        approved_reports: history.approved_reports.len() as i32,
                        linked_subjects_count: history.linked_subjects.len() as i32,
                        first_seen: history.subject.first_seen_at.to_rfc3339(),
                    }).await.ok();
                    Some(history)
                }
                Err(e) => { tracing::warn!("kb history query failed: {e}"); None }
            }
        }
        Ok(None) => None,
        Err(e) => { tracing::warn!("kb subject lookup failed: {e}"); None }
    }
} else { None };
```

### 4. Enrich detective prompt
In `build_detective_input()` (or wherever detective user message is built):
- If `subject_history.is_some()`, append section:
```
## Dữ liệu lịch sử từ hệ thống
- Đối tượng đã được tra cứu {N} lần (lần đầu: {date}, gần nhất: {date})
- Lịch sử đánh giá rủi ro: {risk_history_summary}
- {M} báo cáo từ cộng đồng (đã duyệt): {report_descriptions}
- Liên kết với {K} đối tượng khác: {linked_subjects_list}
```
- Pass `Option<SubjectHistory>` down to detective report builder

### 5. Implement post-ingest
After `set_full_investigation()` cache call, at end of `run_investigation_inner()`:
```rust
if let Some(kb) = knowledge_base.as_ref() {
    // Compute quality score
    let successful = scraped_results.iter().filter(|r| r.success).count();
    let total = scraped_results.len();
    let sources_success_ratio = if total > 0 { successful as f32 / total as f32 } else { 0.0 };
    let total_signals: usize = result.summaries.iter().map(|s| s.risk_signals.len()).sum::<usize>()
        + result.extractions.iter().map(|e| e.risk_signals.len()).sum::<usize>();
    let total_urls: usize = result.summaries.iter().map(|s| s.evidence_urls.len()).sum();
    let evidence_richness = (total_signals + total_urls) as f32 / 10.0;
    let quality_score = compute_quality_score(
        sources_success_ratio, result.confidence,
        evidence_richness.min(1.0), 1.0, // recency=1.0 at ingest
    );
    
    if quality_score >= 0.3 {
        let kb = kb.clone();
        let result = result.clone();
        let query_type_str = investigation.query_type.as_str().to_string();
        tokio::spawn(async move {
            if let Err(e) = ingest_to_knowledge_base(
                &kb, &result, &query_type_str,
                sources_success_ratio, quality_score,
            ).await {
                tracing::warn!("knowledge base ingest failed: {e}");
            }
        });
    }
}
```

### 6. Implement `ingest_to_knowledge_base()` helper
New function in `investigation.rs` (or separate file `src/pipeline/knowledge_base_ingest.rs` if > 100 lines):
```rust
async fn ingest_to_knowledge_base(
    kb: &KnowledgeBase,
    result: &InvestigationResult,
    query_type: &str,
    sources_success_ratio: f32,
    quality_score: f32,
) -> AppResult<()> {
    let risk_score_numeric = risk_level_to_numeric(&result.risk_level);
    let subject_id = kb.upsert_subject(&result.query, query_type).await?;
    let inv_id = kb.insert_investigation(
        subject_id, &result.query, query_type,
        &result.risk_level, risk_score_numeric,
        result.confidence, result.sources_analyzed as i32,
        sources_success_ratio, quality_score,
        result.duration_ms as i64,
        Some(&result.detective_markdown),
    ).await?;
    
    // Insert evidence from summaries
    let mut evidence_inputs = Vec::new();
    for summary in &result.summaries {
        evidence_inputs.push(EvidenceInput {
            subject_id,
            investigation_id: Some(inv_id),
            source: summary.source.clone(),
            evidence_type: "agent_summary".to_string(),
            data: serde_json::to_value(summary)?,
            risk_signals: summary.risk_signals.clone(),
            mentioned_subjects: summary.phone_mentions.clone(),
        });
    }
    
    // Insert evidence from extractions
    for extraction in &result.extractions {
        evidence_inputs.push(EvidenceInput {
            subject_id,
            investigation_id: Some(inv_id),
            source: extraction.url.clone(),
            evidence_type: "agent_extraction".to_string(),
            data: serde_json::to_value(extraction)?,
            risk_signals: extraction.risk_signals.clone(),
            mentioned_subjects: extraction.related_numbers.clone(),
        });
    }
    
    kb.insert_evidence_batch(evidence_inputs).await?;
    kb.recalculate_risk(subject_id).await?;
    Ok(())
}
```

### 7. Update frontend SSE handling
In `frontend/src/lib/types.ts`: add `HistoricalContextEvent` type
In `frontend/src/lib/sse-client.ts`: add `historical_context` event handler
In `frontend/src/routes/+page.svelte`: show historical badge when event received

## Todo List
- [x] Add `HistoricalContext` variant to `InvestigationEvent` in state.rs
- [x] Pass `knowledge_base` param through run_investigation → run_investigation_inner
- [x] Update call site in api/investigate.rs
- [x] Implement pre-query (subject lookup + history) before Phase 1
- [x] Build historical context string/payload for detective prompt enrichment
- [x] Modify detective input builder to include historical section
- [x] Implement post-ingest with quality gate (`quality_score >= 0.3`)
- [x] Implement `ingest_to_knowledge_base()` helper logic in `src/pipeline/knowledge_base.rs`
- [x] Frontend: add HistoricalContextEvent type
- [x] Frontend: handle `historical_context` SSE event
- [x] Frontend: display historical badge/info panel
- [x] Compile check — `cargo check`
- [x] Test: verify pipeline works with KB unavailable (graceful degradation)

## Success Criteria
- Investigation with quality_score >= 0.3 → data persists in KB tables (weighted by quality)
- Investigation with quality_score < 0.3 → NOT ingested (too unreliable)
- Multiple investigations of same subject → risk_score = weighted average by quality_score
- Subject with history → detective report mentions historical context
- SSE emits `historical_context` event when subject found in KB
- Frontend shows "Đã được tra cứu N lần" badge
- Pipeline works normally when KB is None/unavailable
- No performance regression (pre-query < 50ms, post-ingest async)

## Validation Evidence
- 2026-05-24: Booted backend with `DATABASE_URL=` and `REDIS_URL=` empty on dedicated test port and confirmed `/health` returns `{"ok":true,"cache_enabled":false}` while startup logs mark database services disabled instead of crashing.

## Risk Assessment
- **Detective prompt length:** Historical context adds ~200-400 tokens. Monitor max_tokens budget.
- **Ingest failures:** async spawn → fire-and-forget. Log warn, don't retry. Data consistency acceptable for analytics use case.
- **Race condition:** Same query concurrent → duplicate investigations possible. Acceptable — dedup later if needed.

## Security Considerations
- Historical data exposed to detective agent only (not raw to user)
- Frontend receives counts only (investigation_count, approved_reports) — no PII
