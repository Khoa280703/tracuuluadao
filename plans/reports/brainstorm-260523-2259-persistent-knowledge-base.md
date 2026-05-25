# Brainstorm: Persistent Knowledge Base cho Tra Cứu Lừa Đảo

## Problem Statement

Hiện tại hệ thống chỉ có **cache ephemeral** (TTL 1h, cleanup 6h). Mọi data investigation bị mất sau TTL. Cần:
- Build scam database riêng, tích lũy dữ liệu qua thời gian
- Enrich kết quả investigation với historical data
- Cho phép user report (crowdsource, anonymous, admin duyệt)
- Liên kết dữ liệu: phone ↔ bank ↔ URL networks
- Scale: 1K-10K queries/ngày

## Hiện trạng

| Layer | Mô tả | TTL |
|-------|--------|-----|
| `analysis_cache` (PG) | Agent outputs per source | 1h |
| `investigation_cache` (PG) | Full InvestigationResult | 1h |
| Redis Streams | Detective report chunks | configurable |
| **Persistent storage** | **Không có** | — |

**Dữ liệu bị mất:** Toàn bộ scraped results, agent summaries, extractions, detective report, risk signals — tất cả biến mất sau 1h.

## Evaluated Approaches

### Option A: Pure Relational (Normalized)
```
subjects → investigations → evidence → links
(all typed columns, strict schema)
```
- **Pros:** Fast queries, clear schema, easy SQL
- **Cons:** Schema migration mỗi khi thêm scraper/agent mới. Evidence data cấu trúc khác nhau giữa các sources → nhiều nullable columns hoặc phải normalize quá sâu

### Option B: Graph Database (Neo4j / PG AGE)
```
(Phone)-[:LINKED_TO]->(Bank)-[:ASSOCIATED]->(URL)
```
- **Pros:** Natural cho network analysis, traversal queries mạnh
- **Cons:** Thêm infra, learning curve, overkill cho 1K-10K/ngày. PG AGE còn non-mature.

### Option C: Hybrid (PG Relational + JSONB + Materialized Views) ← RECOMMENDED
```
Relational: subjects, subject_links, investigations, user_reports
JSONB: evidence.data (flexible per source)
Materialized Views: risk aggregation, hot subjects
```
- **Pros:** No new infra, flexible evidence schema, fast core queries, proven at scale
- **Cons:** JSONB queries chậm hơn typed columns (mitigated bởi GIN index), materialized views cần refresh strategy

## Recommended Solution: Hybrid Model

### Schema Design

```sql
-- ==========================================
-- CORE: Subjects (đối tượng điều tra)
-- ==========================================
CREATE TABLE subjects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    value TEXT NOT NULL,                    -- "0926408013", "BIDV-123456", "scam.vn"
    subject_type TEXT NOT NULL,             -- "phone", "bank", "url"
    
    -- Aggregated risk (recalculated)
    risk_score REAL DEFAULT 0.0,            -- 0.0 - 1.0
    risk_level TEXT DEFAULT 'unknown',      -- critical/high/medium/low/unknown
    report_count INT DEFAULT 0,
    investigation_count INT DEFAULT 0,
    
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(value, subject_type)
);

CREATE INDEX idx_subjects_value ON subjects(value);
CREATE INDEX idx_subjects_type ON subjects(subject_type);
CREATE INDEX idx_subjects_risk ON subjects(risk_level, risk_score DESC);

-- ==========================================
-- INVESTIGATIONS (mỗi lần điều tra)
-- ==========================================
CREATE TABLE investigations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES subjects(id),
    
    query TEXT NOT NULL,
    query_type TEXT NOT NULL,
    risk_level TEXT NOT NULL,
    confidence REAL NOT NULL,
    sources_analyzed INT NOT NULL,
    duration_ms BIGINT,
    
    detective_report TEXT,                  -- markdown report
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_investigations_subject ON investigations(subject_id);
CREATE INDEX idx_investigations_created ON investigations(created_at DESC);

-- ==========================================
-- EVIDENCE (bằng chứng - flexible schema)
-- ==========================================
CREATE TABLE evidence (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES subjects(id),
    investigation_id UUID REFERENCES investigations(id),
    
    source TEXT NOT NULL,                   -- "checkscam", "google", "user_report", etc.
    evidence_type TEXT NOT NULL,            -- "scraper_result", "agent_summary", "agent_extraction", "user_report"
    
    data JSONB NOT NULL,                    -- flexible: summary, key_facts, entities, etc.
    risk_signals TEXT[] DEFAULT '{}',       -- extracted risk signals
    mentioned_subjects TEXT[] DEFAULT '{}', -- other phones/banks/urls found in this evidence
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_evidence_subject ON evidence(subject_id);
CREATE INDEX idx_evidence_investigation ON evidence(investigation_id);
CREATE INDEX idx_evidence_source ON evidence(source);
CREATE INDEX idx_evidence_data ON evidence USING GIN(data);
CREATE INDEX idx_evidence_mentioned ON evidence USING GIN(mentioned_subjects);

-- ==========================================
-- SUBJECT LINKS (liên kết giữa các đối tượng)
-- ==========================================
CREATE TABLE subject_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_a_id UUID NOT NULL REFERENCES subjects(id),
    subject_b_id UUID NOT NULL REFERENCES subjects(id),
    
    link_type TEXT NOT NULL,                -- "mentioned_together", "same_owner", "redirects_to"
    strength REAL DEFAULT 1.0,             -- số lần xuất hiện cùng nhau
    evidence_ids UUID[] DEFAULT '{}',       -- evidence IDs chứng minh link
    
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(subject_a_id, subject_b_id, link_type),
    CHECK(subject_a_id < subject_b_id)     -- undirected: always store smaller ID first
);

CREATE INDEX idx_links_a ON subject_links(subject_a_id);
CREATE INDEX idx_links_b ON subject_links(subject_b_id);

-- ==========================================
-- USER REPORTS (crowdsource, admin-gated)
-- ==========================================
CREATE TABLE user_reports (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES subjects(id),
    
    reporter_ip_hash TEXT NOT NULL,         -- SHA256 of IP for spam prevention
    description TEXT NOT NULL,
    category TEXT,                          -- "lua_dao", "spam", "gia_mao", etc.
    
    status TEXT NOT NULL DEFAULT 'pending', -- "pending", "approved", "rejected"
    reviewed_by TEXT,                       -- admin identifier
    reviewed_at TIMESTAMPTZ,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_reports_subject ON user_reports(subject_id);
CREATE INDEX idx_user_reports_status ON user_reports(status);

-- ==========================================
-- MATERIALIZED VIEW: Risk Aggregation
-- ==========================================
CREATE MATERIALIZED VIEW subject_risk_overview AS
SELECT 
    s.id,
    s.value,
    s.subject_type,
    s.risk_level,
    s.risk_score,
    COUNT(DISTINCT i.id) AS total_investigations,
    COUNT(DISTINCT e.id) AS total_evidence,
    COUNT(DISTINCT ur.id) FILTER (WHERE ur.status = 'approved') AS approved_reports,
    COUNT(DISTINCT sl.id) AS total_links,
    MAX(i.created_at) AS last_investigated_at,
    ARRAY_AGG(DISTINCT unnested_signal) FILTER (WHERE unnested_signal IS NOT NULL) AS all_risk_signals
FROM subjects s
LEFT JOIN investigations i ON i.subject_id = s.id
LEFT JOIN evidence e ON e.subject_id = s.id
LEFT JOIN user_reports ur ON ur.subject_id = s.id
LEFT JOIN subject_links sl ON sl.subject_a_id = s.id OR sl.subject_b_id = s.id
LEFT JOIN LATERAL unnest(e.risk_signals) AS unnested_signal ON true
GROUP BY s.id;

CREATE UNIQUE INDEX idx_risk_overview_id ON subject_risk_overview(id);
CREATE INDEX idx_risk_overview_value ON subject_risk_overview(value);
```

### Integration Flow

```
User Query: "0926408013" (phone)
    │
    ├─① CHECK KNOWLEDGE BASE
    │   └─ SELECT * FROM subjects WHERE value='0926408013' AND subject_type='phone'
    │   └─ Nếu có: load historical data (investigations, evidence, links, user_reports)
    │
    ├─② RUN PIPELINE (như hiện tại)
    │   └─ Scrapers → Summarizer → URL Assessor → Extractor → Detective
    │   └─ Detective agent NHẬN THÊM context: "Đã được tra cứu N lần, risk history: ..."
    │
    ├─③ INGEST (chỉ khi risk_level >= medium)
    │   └─ UPSERT subject
    │   └─ INSERT investigation
    │   └─ INSERT evidence (per scraper result, per agent output)
    │   └─ DETECT & INSERT subject_links (from mentioned_subjects in evidence)
    │   └─ UPDATE subject risk_score (aggregate recalculation)
    │
    └─④ RESPOND TO USER
        └─ SSE events + historical context:
           "⚠ Số này đã được tra cứu 15 lần, 12 lần risk=high"
           "🔗 Liên kết với: BIDV-9876543, scam-website.com"
```

### Link Detection Logic

```
Khi ingest evidence:
  1. Parse agent summaries → extract phone_mentions, related_numbers
  2. Parse agent extractions → extract entities, related_numbers
  3. For each mentioned phone/bank/url:
     a. UPSERT vào subjects
     b. UPSERT vào subject_links (subject_a ↔ subject_b)
     c. Increment link strength
     d. Append evidence_id
```

### Network Traversal Query (Recursive CTE)

```sql
-- Tìm tất cả subjects liên kết với phone X trong 3 cấp
WITH RECURSIVE network AS (
    -- Seed: subject gốc
    SELECT s.id, s.value, s.subject_type, 0 AS depth
    FROM subjects s WHERE s.value = '0926408013' AND s.subject_type = 'phone'
    
    UNION
    
    -- Traverse links
    SELECT 
        CASE WHEN sl.subject_a_id = n.id THEN s2.id ELSE s1.id END,
        CASE WHEN sl.subject_a_id = n.id THEN s2.value ELSE s1.value END,
        CASE WHEN sl.subject_a_id = n.id THEN s2.subject_type ELSE s1.subject_type END,
        n.depth + 1
    FROM network n
    JOIN subject_links sl ON sl.subject_a_id = n.id OR sl.subject_b_id = n.id
    JOIN subjects s1 ON s1.id = sl.subject_a_id
    JOIN subjects s2 ON s2.id = sl.subject_b_id
    WHERE n.depth < 3
)
SELECT DISTINCT ON (id) * FROM network ORDER BY id, depth;
```

### Enrichment cho Detective Agent

```
Khi build detective prompt, thêm section:

## Dữ liệu lịch sử
- Số này đã được tra cứu 15 lần (lần đầu: 2025-01-15, gần nhất: 2026-05-20)
- Risk history: 12/15 lần = high, 2 = medium, 1 = unknown
- 5 user reports (3 approved): "lừa đảo chuyển khoản", "mạo danh ngân hàng"
- Liên kết với:
  - Bank: BIDV-9876543 (risk: high, 8 reports)
  - Phone: 0987654321 (risk: medium, 3 reports)
  - URL: scam-website.com (risk: critical, 20 reports)

→ AI có thêm context để đưa ra đánh giá chính xác hơn
```

### Materialized View Refresh Strategy

```
Option 1: Periodic (RECOMMENDED cho giai đoạn đầu)
  - Cron job mỗi 15 phút: REFRESH MATERIALIZED VIEW CONCURRENTLY subject_risk_overview
  - Simple, predictable, không block reads

Option 2: On-demand (scale lớn hơn)
  - Trigger refresh sau mỗi batch ingest
  - Hoặc lazy refresh: check age, refresh nếu > 5 phút
```

### Crowdsource: User Report Flow

```
User submit report → status='pending' → Admin dashboard review
  ├─ Approve → status='approved' → recalculate subject risk_score
  └─ Reject → status='rejected' → không ảnh hưởng risk

Anti-spam:
  - Rate limit: 5 reports/IP/ngày
  - IP hash (SHA256) — không lưu IP thật
  - Duplicate detection: same IP + same subject trong 24h → reject
```

## Implementation Considerations

### Phase 1: Core Knowledge Base (ưu tiên)
1. Tạo schema migration (subjects, investigations, evidence, subject_links)
2. Implement `KnowledgeBase` service (Rust module)
3. Hook vào pipeline: ingest sau investigation hoàn thành (risk >= medium)
4. Query historical data trước pipeline, feed vào detective agent
5. Frontend: hiển thị historical badge trên kết quả

### Phase 2: Link Detection & Network
1. Parse evidence cho mentioned subjects
2. Auto-create links
3. Network traversal API endpoint
4. Frontend: hiển thị linked subjects

### Phase 3: Crowdsource
1. User report API endpoint
2. Admin review dashboard (simple)
3. Risk recalculation khi approve
4. Rate limiting & spam prevention

### Backward Compatibility
- Cache layer (analysis_cache, investigation_cache) **giữ nguyên** — vẫn dùng cho short-term dedup
- Knowledge base là layer mới, độc lập
- Pipeline flow không thay đổi, chỉ thêm pre-query (historical) và post-ingest (save)

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Storage growth nhanh | Chỉ ingest risk >= medium; archive old evidence sau 1 năm |
| JSONB query chậm | GIN index trên evidence.data; materialized views cho aggregation |
| Spam user reports | IP rate limit + admin gate + duplicate detection |
| Data quality (scrapers thay đổi) | evidence_type + source tracking; versioning implicit qua created_at |
| Link explosion (false positives) | Link strength threshold; chỉ tạo link khi >= 2 evidence cùng mention |

## Success Metrics

- Knowledge base accumulates > 1000 unique subjects trong tháng đầu
- > 30% investigations được enrich bởi historical data
- Detective agent confidence tăng khi có historical context
- User reports: > 50 approved reports/tháng
- Link network phát hiện ít nhất 1 scam cluster/tuần

## Next Steps

1. **Schema migration** — tạo tables trong PostgreSQL
2. **`KnowledgeBase` Rust module** — CRUD + ingest + query + link detection
3. **Pipeline integration** — pre-query + post-ingest hooks
4. **Detective prompt enrichment** — inject historical context
5. **Frontend UI** — historical badges, linked subjects, report form
6. **Admin dashboard** — review user reports (có thể simple CLI trước)
