# Phase 4: Crowdsource & Admin API

## Context
- [Phase 1](./phase-01-schema-and-knowledge-base-service.md) — `user_reports` table phải tồn tại
- Independent of Phase 3 — có thể song song sau Phase 1

## Overview
- **Priority:** P2
- **Status:** Complete
- **Progress:** 100%
- **Effort:** 4h
- API cho user submit scam report (anonymous, admin duyệt). Admin API review/approve/reject. Rate limiting chống spam. Risk recalculation khi approve.

## Key Insights
- Anonymous: không cần auth system, dùng IP hash (SHA256) chống spam
- Admin auth: simple API key header cho MVP (không cần full auth system)
- Rate limit: 5 reports/IP/ngày — implement tại DB query level, không cần middleware
- Approved reports → recalculate subject risk → ảnh hưởng future investigations

## Requirements

### Functional
- `POST /api/reports` — submit report (anonymous, rate-limited)
- `GET /api/admin/reports?status=pending` — list reports for review
- `POST /api/admin/reports/:id/approve` — approve report → recalculate risk
- `POST /api/admin/reports/:id/reject` — reject report
- Rate limit: max 5 reports per IP hash per 24h
- Duplicate detection: same IP + same subject within 24h → reject

### Non-functional
- Admin endpoints protected by `X-Admin-Key` header (env: `ADMIN_API_KEY`)
- Report description max 2000 chars
- Subject auto-created if not exists (upsert on report submit)

## Architecture

```
User → POST /api/reports
  ├─ Extract IP → SHA256 hash
  ├─ Rate limit check (DB query)
  ├─ Duplicate check (DB query)
  ├─ Upsert subject
  ├─ Insert user_report (status=pending)
  └─ 201 Created

Admin → GET /api/admin/reports?status=pending
  └─ X-Admin-Key header check
  └─ List pending reports with subject info

Admin → POST /api/admin/reports/:id/approve
  └─ X-Admin-Key check
  └─ UPDATE status=approved, reviewed_by, reviewed_at
  └─ Recalculate subject risk
  └─ Update subject report_count
```

## Related Code Files

### Files to Create
- `src/api/reports.rs` — user report submission endpoint
- `src/api/admin.rs` — admin review endpoints

### Files to Modify
- `src/knowledge_base/mod.rs` — add report CRUD methods
- `src/api/mod.rs` — add report + admin routes
- `src/config.rs` — add `admin_api_key: Option<String>`
- `frontend/src/routes/+page.svelte` — add report form (after investigation)
- `frontend/src/lib/types.ts` — add report types

## Implementation Steps

### 1. Add config
In `src/config.rs`:
```rust
pub admin_api_key: Option<String>,
// in from_env():
admin_api_key: optional_env_var("ADMIN_API_KEY"),
```

### 2. Add KnowledgeBase report methods
In `src/knowledge_base/mod.rs`:
```rust
pub async fn check_report_rate_limit(
    &self, ip_hash: &str, max_per_day: i32,
) -> AppResult<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_reports
         WHERE reporter_ip_hash = $1 AND created_at > NOW() - INTERVAL '24 hours'"
    ).bind(ip_hash).fetch_one(&self.pool).await?;
    Ok(count < max_per_day as i64)
}

pub async fn check_duplicate_report(
    &self, ip_hash: &str, subject_id: Uuid,
) -> AppResult<bool> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_reports
         WHERE reporter_ip_hash = $1 AND subject_id = $2
         AND created_at > NOW() - INTERVAL '24 hours')"
    ).bind(ip_hash).bind(subject_id).fetch_one(&self.pool).await?;
    Ok(!exists) // true = no duplicate
}

pub async fn insert_user_report(
    &self, subject_id: Uuid, ip_hash: &str,
    description: &str, category: Option<&str>,
) -> AppResult<Uuid> { ... }

pub async fn list_reports_by_status(
    &self, status: &str, limit: i32, offset: i32,
) -> AppResult<Vec<UserReportWithSubject>> { ... }

pub async fn update_report_status(
    &self, report_id: Uuid, status: &str, reviewed_by: &str,
) -> AppResult<bool> { ... }
```

### 3. Create `src/api/reports.rs`
```rust
#[derive(Deserialize)]
pub struct SubmitReportRequest {
    pub value: String,          // phone/bank/url value
    pub subject_type: String,   // "phone", "bank", "url"
    pub description: String,    // max 2000 chars
    pub category: Option<String>,
}

pub async fn submit_report(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<SubmitReportRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let kb = state.knowledge_base.as_ref()
        .ok_or(AppError::Config("knowledge base not available".into()))?;
    
    // Validate
    if body.description.len() > 2000 {
        return Err(AppError::Config("description too long (max 2000)".into()));
    }
    
    // IP hash
    let ip_hash = sha256_hex(&addr.ip().to_string());
    
    // Rate limit
    if !kb.check_report_rate_limit(&ip_hash, 5).await? {
        return Err(AppError::Config("rate limit exceeded (max 5/day)".into()));
    }
    
    // Upsert subject
    let subject_id = kb.upsert_subject(&body.value, &body.subject_type).await?;
    
    // Duplicate check
    if !kb.check_duplicate_report(&ip_hash, subject_id).await? {
        return Err(AppError::Config("duplicate report for this subject today".into()));
    }
    
    let report_id = kb.insert_user_report(
        subject_id, &ip_hash, &body.description, body.category.as_deref(),
    ).await?;
    
    Ok((StatusCode::CREATED, Json(json!({ "id": report_id }))))
}
```

### 4. Create `src/api/admin.rs`
```rust
async fn check_admin_key(
    headers: &HeaderMap, config: &AppConfig,
) -> AppResult<()> {
    let key = config.admin_api_key.as_ref()
        .ok_or(AppError::Config("admin API not configured".into()))?;
    let provided = headers.get("x-admin-key")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Config("missing X-Admin-Key header".into()))?;
    if provided != key {
        return Err(AppError::Config("invalid admin key".into()));
    }
    Ok(())
}

pub async fn list_pending_reports(...) -> AppResult<Json<Vec<UserReportWithSubject>>> { ... }
pub async fn approve_report(...) -> AppResult<Json<Value>> {
    // update status → recalculate_risk → update report_count
}
pub async fn reject_report(...) -> AppResult<Json<Value>> { ... }
```

### 5. Add routes to `src/api/mod.rs`
```rust
.route("/api/reports", post(reports::submit_report))
.route("/api/admin/reports", get(admin::list_pending_reports))
.route("/api/admin/reports/:id/approve", post(admin::approve_report))
.route("/api/admin/reports/:id/reject", post(admin::reject_report))
```
- Need to pass `AppConfig` into state or extract from env in handlers

### 6. Frontend: report form
After investigation shows risk result, add "Báo cáo lừa đảo" button:
- Simple form: description textarea + category dropdown
- POST to `/api/reports`
- Show success/error message
- No auth required

## Todo List
- [x] Add `admin_api_key` to AppConfig
- [x] Add report CRUD methods to KnowledgeBase (rate limit, duplicate, insert, list, update)
- [x] Create `src/api/reports.rs` — submit endpoint with rate limiting
- [x] Create `src/api/admin.rs` — list/approve/reject endpoints with admin key auth
- [x] Add routes to api/mod.rs
- [x] Add `ConnectInfo` extractor setup (axum `into_make_service_with_connect_info`)
- [x] Frontend: add report form component
- [x] Frontend: add report submission API call
- [x] Compile check — `cargo check`
- [x] Test: rate limit blocks 6th report from same IP
- [x] Test: approve recalculates risk

## Success Criteria
- Anonymous user can submit report → status=pending in DB
- 6th report from same IP in 24h → rejected
- Duplicate (same IP + subject in 24h) → rejected
- Admin with valid key can list/approve/reject
- Approve → subject risk recalculated
- Invalid/missing admin key → 401 error
- Frontend report form works after investigation

## Validation Evidence
- 2026-05-24: `POST /api/reports` accepted first report and rejected duplicate report for the same subject within 24h.
- 2026-05-24: `GET /api/admin/reports` without `X-Admin-Key` returned `401 Unauthorized`.
- 2026-05-24: `POST /api/admin/reports/{id}/approve` changed subject `risk_level` from `unknown` to `low` and `report_count` from `0` to `1`.
- 2026-05-24: After five accepted reports from the same client IP, the sixth report returned `rate limit exceeded (max 5 reports per day)`.

## Risk Assessment
- **Admin key in env:** Simple but sufficient for MVP. Upgrade to JWT/session auth if admin team grows.
- **IP hash collision:** SHA256 collision practically impossible. IP behind NAT = multiple users share limit. Acceptable tradeoff.
- **ConnectInfo behind proxy:** If behind Caddy/Nginx, need `X-Forwarded-For` header parsing instead of `ConnectInfo`. Check Caddy config passes real IP.

## Security Considerations
- IP stored as SHA256 hash — no PII
- Admin key compared with constant-time comparison (防 timing attack) — use `subtle::ConstantTimeEq` or simple `==` (acceptable for MVP)
- Report description sanitized: no HTML rendering, stored as plain text
- Rate limit at DB level — resistant to distributed bypass within same IP
