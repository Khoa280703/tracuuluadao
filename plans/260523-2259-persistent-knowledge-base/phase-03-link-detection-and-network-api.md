# Phase 3: Link Detection & Network API

## Context
- [Phase 2](./phase-02-pipeline-integration.md) — ingest phải hoạt động trước để populate links
- Link data từ: `AgentSummary.phone_mentions`, `AgentExtraction.related_numbers`, `evidence.mentioned_subjects`

## Overview
- **Priority:** P2
- **Status:** Complete
- **Progress:** 100%
- **Effort:** 4h
- Auto-detect links giữa subjects khi ingest evidence. Expose network traversal API. Frontend hiển thị linked subjects.

## Key Insights
- Link sources: `phone_mentions` (summaries) + `related_numbers` (extractions) — đã có sẵn trong data model
- mentioned_subjects trong evidence table cũng chứa raw values → parse để tạo links
- Recursive CTE cho traversal 2-3 cấp — PostgreSQL native, không cần graph DB
- False positive risk: chỉ tạo link khi value match phone/bank/url regex pattern

## Requirements

### Functional
- Auto-detect: khi ingest evidence, parse mentioned_subjects → upsert subjects + links
- Network API: `GET /api/subjects/:value/network?type=phone&depth=2` → linked subjects graph
- Subject lookup API: `GET /api/subjects/:value?type=phone` → subject details + history
- Frontend: hiển thị linked subjects trong investigation result

### Non-functional
- Link detection runs within post-ingest (same async spawn)
- Network query depth capped at 3 (prevent expensive traversals)
- Response < 100ms for depth=2 queries

## Architecture

```
Post-ingest (Phase 2)
  └─ [NEW] detect_and_create_links()
      ├─ For each summary: parse phone_mentions
      ├─ For each extraction: parse related_numbers
      ├─ Regex classify: phone? bank? url?
      ├─ kb.upsert_subject() for each mentioned value
      └─ kb.upsert_subject_link(source_subject, mentioned_subject)

API Layer
  ├─ GET /api/subjects/:value?type=phone     → SubjectDetail
  └─ GET /api/subjects/:value/network?type=phone&depth=2 → NetworkGraph
```

## Related Code Files

### Files to Create
- `src/api/subjects.rs` — subject lookup + network API handlers

### Files to Modify
- `src/pipeline/investigation.rs` — add `detect_and_create_links()` in post-ingest
- `src/knowledge_base/mod.rs` — add `get_network()` method with recursive CTE
- `src/api/mod.rs` — add routes for subjects API
- `frontend/src/lib/types.ts` — add LinkedSubject, NetworkGraph types
- `frontend/src/routes/+page.svelte` — display linked subjects section

## Implementation Steps

### 1. Add `detect_and_create_links()` to pipeline
In `ingest_to_knowledge_base()` (Phase 2), after evidence insert:
```rust
async fn detect_and_create_links(
    kb: &KnowledgeBase,
    source_subject_id: Uuid,
    result: &InvestigationResult,
) -> AppResult<()> {
    let mut mentioned_values: HashSet<(String, String)> = HashSet::new(); // (value, type)
    
    for summary in &result.summaries {
        for phone in &summary.phone_mentions {
            if is_phone_number(phone) && phone != &result.query {
                mentioned_values.insert((phone.clone(), "phone".into()));
            }
        }
    }
    for extraction in &result.extractions {
        for number in &extraction.related_numbers {
            let subject_type = classify_value(number);
            if let Some(st) = subject_type {
                mentioned_values.insert((number.clone(), st));
            }
        }
    }
    
    for (value, subject_type) in mentioned_values {
        let linked_id = kb.upsert_subject(&value, &subject_type).await?;
        kb.upsert_subject_link(
            source_subject_id, linked_id,
            "mentioned_together", None,
        ).await?;
    }
    Ok(())
}

fn classify_value(value: &str) -> Option<String> {
    if PHONE_REGEX.is_match(value) { Some("phone".into()) }
    else if BANK_REGEX.is_match(value) { Some("bank".into()) }
    else if value.starts_with("http") { Some("url".into()) }
    else { None }
}
```

### 2. Add `get_network()` to KnowledgeBase
```rust
pub async fn get_network(
    &self, subject_id: Uuid, max_depth: i32,
) -> AppResult<Vec<NetworkNode>> {
    let depth = max_depth.min(3); // cap at 3
    sqlx::query_as::<_, NetworkNode>(
        "WITH RECURSIVE network AS (
            SELECT s.id, s.value, s.subject_type, s.risk_level, s.risk_score, 0 AS depth
            FROM subjects s WHERE s.id = $1
            UNION
            SELECT
                CASE WHEN sl.subject_a_id = n.id THEN s2.id ELSE s1.id END,
                CASE WHEN sl.subject_a_id = n.id THEN s2.value ELSE s1.value END,
                CASE WHEN sl.subject_a_id = n.id THEN s2.subject_type ELSE s1.subject_type END,
                CASE WHEN sl.subject_a_id = n.id THEN s2.risk_level ELSE s1.risk_level END,
                CASE WHEN sl.subject_a_id = n.id THEN s2.risk_score ELSE s1.risk_score END,
                n.depth + 1
            FROM network n
            JOIN subject_links sl ON sl.subject_a_id = n.id OR sl.subject_b_id = n.id
            JOIN subjects s1 ON s1.id = sl.subject_a_id
            JOIN subjects s2 ON s2.id = sl.subject_b_id
            WHERE n.depth < $2
        )
        SELECT DISTINCT ON (id) id, value, subject_type, risk_level, risk_score, depth
        FROM network ORDER BY id, depth"
    )
    .bind(subject_id)
    .bind(depth)
    .fetch_all(&self.pool)
    .await
    .map_err(Into::into)
}
```

### 3. Create `src/api/subjects.rs`
```rust
#[derive(Deserialize)]
pub struct SubjectParams {
    pub r#type: Option<String>,  // phone, bank, url
    pub depth: Option<i32>,      // for network endpoint
}

pub async fn get_subject(
    Path(value): Path<String>,
    Query(params): Query<SubjectParams>,
    State(state): State<AppState>,
) -> AppResult<Json<SubjectDetail>> { ... }

pub async fn get_network(
    Path(value): Path<String>,
    Query(params): Query<SubjectParams>,
    State(state): State<AppState>,
) -> AppResult<Json<NetworkGraph>> { ... }
```

### 4. Add routes to `src/api/mod.rs`
```rust
.route("/api/subjects/:value", get(subjects::get_subject))
.route("/api/subjects/:value/network", get(subjects::get_network))
```

### 5. Frontend updates
- `types.ts`: add `LinkedSubject`, `NetworkGraph` interfaces
- `sse-client.ts`: no change (links come from API, not SSE)
- `+page.svelte`: after investigation complete, if risk >= medium, fetch `/api/subjects/{query}/network` and display linked subjects panel

## Todo List
- [x] Implement `detect_and_create_links()` with value classification regex
- [x] Call link detection in `ingest_to_knowledge_base()` after evidence insert
- [x] Add `get_network()` recursive CTE method to KnowledgeBase
- [x] Create `src/api/subjects.rs` with get_subject + get_network handlers
- [x] Add subject routes to api/mod.rs router
- [x] Add NetworkNode struct to knowledge_base/models.rs
- [x] Frontend: add types + fetch + display linked subjects
- [x] Compile check — `cargo check`

## Success Criteria
- Phone mentioned in summary → auto-creates subject + link
- `/api/subjects/0926408013/network?type=phone&depth=2` returns linked graph
- Frontend displays linked subjects after investigation
- Depth capped at 3 — no runaway queries
- Link strength increments on repeat mentions

## Risk Assessment
- **False positive links:** Phone regex may match non-phone numbers. Use strict Vietnamese phone pattern: `^(0[0-9]{9}|84[0-9]{9,10})$`
- **Recursive CTE performance:** At depth=3 with many links could be slow. EXPLAIN ANALYZE after 1K+ subjects. Add timeout if needed.
