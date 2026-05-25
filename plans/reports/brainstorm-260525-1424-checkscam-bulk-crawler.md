# Brainstorm: Checkscam Bulk Crawler + Media Architecture

**Date:** 2026-05-25
**Status:** Agreed

## Problem Statement

Nền tảng cần seed data trước khi go-live. checkscam.vn có hàng ngàn bài báo cáo lừa đảo (WordPress site). Cào toàn bộ data về, map vào KB schema hiện tại, lưu cả ảnh bằng chứng.

## Agreed Solution

### 1. Bulk Crawler — Rust CLI Script

**Strategy: Hybrid (REST API + Browser Sidecar)**
- **Phase 1 — REST API bulk:** `GET /wp-json/wp/v2/posts?per_page=100&page=N` → lấy toàn bộ post IDs + title + content.rendered (HTML)
- **Phase 2 — Sidecar detail:** Cho mỗi post, gọi sidecar để lấy full HTML (bypasses Cloudflare) → parse chi tiết + extract ảnh URLs
- **Fallback:** Nếu sidecar unavailable, parse trực tiếp từ REST `content.rendered`

**Runtime:** CLI binary riêng (`cargo run --bin checkscam-crawler`), không ảnh hưởng server chính.

**Reuse từ codebase hiện tại:**
- `parse_detail_report()` — regex extract owner, account, bank, warning, report_count
- `KnowledgeBase` service — upsert_subject, insert_evidence_batch
- `HttpClientFactory` — HTTP client with TLS impersonation
- Sidecar client — `POST http://127.0.0.1:4417/checkscam/detail`

### 2. Data Mapping (KB Schema — No Change)

| Checkscam field | KB table | KB field |
|---|---|---|
| Phone/bank extracted from post | `subjects` | `value`, `subject_type` |
| Parsed content (JSONB) | `evidence` | `data` |
| `"checkscam_crawl"` | `evidence` | `source` |
| `"external_report"` | `evidence` | `evidence_type` |
| Warning text → signals | `evidence` | `risk_signals` |
| Related numbers in same post | `evidence` | `mentioned_subjects` |
| Evidence images | `media` (new) | `file_path`, `original_url` |

**Risk assignment (in script):**
- 1-2 checkscam reports → `medium` (risk_score 0.5)
- 3-4 reports → `high` (risk_score 0.7)
- 5+ reports → `critical` (risk_score 0.9)

**No fake investigations:** `investigations` table only tracks real pipeline runs. Crawled evidence has `investigation_id = NULL`.

### 3. Media Table (New — Shared Architecture)

```sql
CREATE TABLE media (
    id UUID PRIMARY KEY,
    entity_type TEXT NOT NULL,  -- 'evidence', 'user_report', 'investigation'
    entity_id UUID NOT NULL,
    file_path TEXT NOT NULL,    -- relative: evidence/{subject_id}/{hash}.jpg
    original_url TEXT,          -- source URL if crawled
    content_type TEXT,          -- image/jpeg, image/png
    file_size_bytes BIGINT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX idx_media_entity ON media(entity_type, entity_id);
```

**Filesystem:**
```
data/media/
├── evidence/{subject_id}/{hash}.jpg
├── reports/{report_id}/{hash}.jpg
└── investigations/{inv_id}/{hash}.jpg
```

**Used by:** checkscam crawl, user reports (upload), future investigation screenshots.

### 4. Crawler Flow

```
CLI start
├─ Connect DB (DATABASE_URL)
├─ Phase 1: REST API pagination
│   └─ GET /wp-json/wp/v2/posts?per_page=100&page=1..N
│   └─ Collect all post URLs + basic metadata
│
├─ Phase 2: Detail fetch (concurrent, rate-limited)
│   ├─ For each post URL:
│   │   ├─ Sidecar detail fetch (or fallback to REST content)
│   │   ├─ parse_detail_report() → extract entities
│   │   ├─ Extract image URLs from HTML
│   │   └─ Download images → data/media/evidence/{subject_id}/
│   │
│   ├─ Upsert subject(s) per post
│   ├─ Insert evidence (investigation_id = NULL)
│   ├─ Insert media records
│   └─ Set initial risk based on report_count
│
├─ Phase 3: Link detection
│   └─ mentioned_subjects → upsert subject_links
│
└─ Summary: {total_posts, subjects_created, evidence_inserted, images_downloaded}
```

**Rate limiting:** 2-4 concurrent requests, 200ms delay between pages. Respectful crawling.

**Idempotency:** Upsert on subjects (UNIQUE value+type). Evidence dedup by source+url combo.

## Implementation Estimate
- **Media table + schema:** 1h
- **Crawler CLI binary:** 4-6h (REST pagination, sidecar integration, image download, DB insert)
- **Link detection pass:** 1h
- **Testing + tuning:** 2h
- **Total:** ~8-10h

## Files to Create/Modify
- `src/bin/checkscam-crawler.rs` — CLI entry point
- `src/knowledge_base/schema.rs` — add media table
- `src/knowledge_base/media.rs` — media CRUD methods
- `Cargo.toml` — add [[bin]] entry

## Risk Assessment
- **Cloudflare blocks REST API:** Unlikely (WP REST usually not behind CF challenge). Fallback: sidecar for everything (slower).
- **Rate limiting from checkscam:** Crawl respectfully (2-4 concurrent, delays). If blocked, reduce concurrency.
- **Duplicate data:** Upsert + dedup logic handles re-runs safely.
- **Disk space for images:** Estimate ~50-100MB for thousands of posts (most images are screenshots).
- **Legal:** Publicly available data, used for anti-scam public service. Low risk.

## Success Criteria
- All checkscam posts crawled and parsed → subjects + evidence in DB
- Images downloaded and referenced in media table
- Risk levels assigned based on report_count
- mentioned_subjects populated → link detection works
- Re-runnable (idempotent) without duplicates
- Pipeline pre-query finds crawled subjects → enriched investigations
