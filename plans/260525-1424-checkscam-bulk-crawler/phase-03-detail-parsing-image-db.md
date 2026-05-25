---
phase: 3
title: "Detail Parsing + Image Download + DB Insert"
status: pending
effort: 3h
depends_on: [phase-01, phase-02]
---

# Phase 3: Detail Parsing + Image Download + DB Insert

## Context Links

- [Phase 1 — Media CRUD](./phase-01-media-table-schema-crud.md)
- [Phase 2 — CLI + pagination](./phase-02-crawler-cli-rest-pagination.md)
- [parse_detail_report()](../../src/scrapers/checkscam.rs) — lines 484-636
- [KnowledgeBase subjects](../../src/knowledge_base/subjects.rs) — upsert_subject, insert_evidence_batch
- [EvidenceInput model](../../src/knowledge_base/models.rs) — lines 74-83

## Overview

Implement `process_posts()` in the crawler binary. For each post: fetch detail HTML, parse entities, extract + download evidence images, upsert subjects, insert evidence records, insert media records, set initial risk.

## Key Insights

- **Sidecar first, REST fallback:** Sidecar (`POST /checkscam/detail`) gives full rendered HTML. If unavailable, use `content.rendered` from the REST API response (already fetched in Phase 2). REST content is often sufficient for regex parsing but may miss dynamic content.
- **Entity extraction reuse:** `parse_detail_report()` already extracts owner, account, bank, warning, report_count. Returns `ScrapedReport` with title + content string. We need to re-parse the content string to extract structured fields for subject upsert.
- **Better approach:** Extract entities directly from HTML in the crawler using the same regexes, rather than parsing `ScrapedReport.content`. This avoids double-parsing. Copy/adapt the regex extraction logic into a `CrawledPost` struct.
- **Image extraction:** Parse `<img>` tags from detail HTML. Filter to checkscam.vn domain images only (skip external CDN, tracking pixels). Download with SHA256 hash naming for dedup.
- **Evidence dedup:** Before inserting evidence, check if evidence with same source + URL combo exists. Use a SQL query: `SELECT EXISTS(... WHERE source = 'checkscam_crawl' AND data->>'source_url' = $1)`.
- **Concurrency:** `tokio::sync::Semaphore` limits concurrent detail fetches. Add delay between batches.

## Requirements

**Functional:**
- Fetch detail HTML via sidecar (fallback: REST content.rendered)
- Extract entities: phone numbers, bank accounts, owner names, bank names, warning text, report_count
- Determine subject_type for each entity: phone → "phone", bank account → "bank"
- Extract image URLs from HTML `<img>` tags
- Download images to `data/media/evidence/{subject_id}/{sha256}.{ext}`
- Upsert subjects, insert evidence (investigation_id=NULL, source="checkscam_crawl"), insert media
- Resume support: skip posts already processed

**Non-functional:**
- Concurrency: Semaphore-limited (default 3)
- Rate limiting: delay between detail fetches
- Error isolation: one failed post doesn't stop the crawl

## Architecture

```
process_posts()
├── Check sidecar availability (single health check)
├── For each CrawlPostRecord (concurrent via Semaphore):
│   ├── Resume check: skip if evidence exists for this URL
│   ├── Fetch detail HTML
│   │   ├── Try sidecar: POST /checkscam/detail { url }
│   │   └── Fallback: use post.content.rendered from REST
│   │
│   ├── Parse entities from HTML
│   │   ├── phone_regex → Vec<String>
│   │   ├── account_regex → Vec<String>
│   │   ├── owner from parse_detail_report
│   │   ├── bank from collapsed text
│   │   ├── warning text
│   │   └── report_count (integer)
│   │
│   ├── Extract image URLs from <img> tags
│   │   └── Filter: checkscam.vn domain, image extensions
│   │
│   ├── For each entity (phone/bank_account):
│   │   ├── kb.upsert_subject(value, type) → subject_id
│   │   ├── Build EvidenceInput {
│   │   │     subject_id, investigation_id: None,
│   │   │     source: "checkscam_crawl",
│   │   │     evidence_type: "external_report",
│   │   │     data: { source_url, title, owner, bank, warning, report_count, date },
│   │   │     risk_signals: [...],
│   │   │     mentioned_subjects: [other entities in same post]
│   │   │   }
│   │   ├── kb.insert_evidence_batch([evidence])
│   │   │
│   │   ├── Download images → data/media/evidence/{subject_id}/
│   │   └── kb.insert_media(...) for each image
│   │
│   └── Set initial risk based on report_count
│
└── Return CrawlStats
```

## Related Code Files

**Modify:**
- `src/bin/checkscam_crawler.rs` — replace `process_posts()` placeholder with full implementation

**Read (reuse patterns from):**
- `src/scrapers/checkscam.rs` — regex patterns, `fetch_detail_via_sidecar`, `parse_detail_report`
- `src/knowledge_base/subjects.rs` — `upsert_subject`, `insert_evidence_batch`
- `src/knowledge_base/media.rs` — `insert_media`, `media_exists_by_url`
- `src/knowledge_base/risk.rs` — `numeric_to_risk_level`

## Implementation Steps

### Step 1: Add helper structs and regex constants

Add to `checkscam_crawler.rs`:

```rust
use std::collections::HashSet;
use std::path::PathBuf;

use futures::stream::{self, StreamExt};
use regex::Regex;
use scraper::{Html, Selector};
use sha2::{Sha256, Digest};
use tokio::sync::Semaphore;
use uuid::Uuid;

use tracuuluadao::knowledge_base::EvidenceInput;
use tracuuluadao::scrapers::checkscam::{parse_detail_report, SIDECAR_BASE_URL};

/// Entities extracted from a single checkscam post.
#[derive(Debug, Default)]
struct ExtractedEntities {
    phones: Vec<String>,
    bank_accounts: Vec<String>,
    owner: Option<String>,
    bank: Option<String>,
    warning: Option<String>,
    report_count: Option<u32>,
    image_urls: Vec<String>,
    source_url: String,
    title: String,
    date: Option<String>,
}
```

### Step 2: Implement sidecar health check

```rust
async fn check_sidecar_available() -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    client
        .get(format!("{SIDECAR_BASE_URL}/health"))
        .send()
        .await
        .is_ok()
}
```

### Step 3: Implement detail HTML fetching

```rust
async fn fetch_detail_html(
    sidecar_client: Option<&reqwest::Client>,
    rest_content_rendered: &str,
    post_url: &str,
) -> String {
    // Try sidecar first
    if let Some(sidecar) = sidecar_client {
        match sidecar
            .post(format!("{SIDECAR_BASE_URL}/checkscam/detail"))
            .json(&serde_json::json!({ "url": post_url }))
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if body["ok"].as_bool().unwrap_or(false) {
                        if let Some(html) = body["html"].as_str() {
                            if !html.is_empty() {
                                return html.to_owned();
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!(url = %post_url, error = %e, "sidecar detail fetch failed");
            }
        }
    }

    // Fallback: REST content.rendered (already HTML)
    rest_content_rendered.to_owned()
}
```

### Step 4: Implement entity extraction from HTML

```rust
fn extract_entities(post: &CrawlPostRecord, detail_html: &str) -> ExtractedEntities {
    let phone_re = Regex::new(r"\b(0[1-9]\d{8,9})\b").unwrap();
    let account_re = Regex::new(r"STK:\s*([0-9*]{6,})").unwrap();
    let owner_re = Regex::new(r"Chủ tk:\s*(.*?)\s*STK:").unwrap();
    let bank_re = Regex::new(
        r"Ngân hàng:\s*(.*?)\s*(?:Hạng mục:|Ảnh chụp bằng chứng:|Nội dung cảnh báo:)"
    ).unwrap();
    let warning_re = Regex::new(
        r"Nội dung cảnh báo:\s*(.*?)\s*(?:💬|_{3,}|Bình luận Copy link|LỊCH SỬ PHẢN ÁNH)"
    ).unwrap();
    let report_count_re = Regex::new(r"đã bị cảnh báo\s+(\d+)\s+lần").unwrap();
    let quick_summary_re = Regex::new(
        r"Họ Tên:\s*(.*?)\s*,\s*SĐT:\s*(.*?)\s*,\s*STK:\s*([0-9*]+)\s*,\s*Ngân hàng:\s*(.*?)\s*,"
    ).unwrap();

    let document = Html::parse_document(detail_html);
    let collapsed = document.root_element().text().collect::<Vec<_>>().join(" ");
    let collapsed = collapsed.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut phones = HashSet::new();
    let mut bank_accounts = HashSet::new();

    // Extract from quick summary
    if let Some(caps) = quick_summary_re.captures(&collapsed) {
        if let Some(phone) = caps.get(2) {
            let cleaned: String = phone.as_str().chars().filter(|c| c.is_ascii_digit()).collect();
            if cleaned.len() >= 9 { phones.insert(cleaned); }
        }
        if let Some(acct) = caps.get(3) {
            let cleaned = acct.as_str().trim().to_owned();
            if cleaned.len() >= 6 { bank_accounts.insert(cleaned); }
        }
    }

    // Extract all phone numbers from text
    for cap in phone_re.captures_iter(&collapsed) {
        if let Some(m) = cap.get(1) {
            phones.insert(m.as_str().to_owned());
        }
    }

    // Extract all bank accounts
    for cap in account_re.captures_iter(&collapsed) {
        if let Some(m) = cap.get(1) {
            bank_accounts.insert(m.as_str().to_owned());
        }
    }

    let owner = owner_re.captures(&collapsed)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().trim_matches('.').to_owned())
        .filter(|v| !v.is_empty());

    let bank = bank_re.captures(&collapsed)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_owned())
        .filter(|v| !v.is_empty());

    let warning = warning_re.captures(&collapsed)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_owned())
        .filter(|v| !v.is_empty());

    let report_count = report_count_re.captures(&collapsed)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok());

    // Extract image URLs
    let img_selector = Selector::parse("img[src]").expect("valid selector");
    let image_urls: Vec<String> = document
        .select(&img_selector)
        .filter_map(|el| el.value().attr("src"))
        .filter(|src| {
            src.contains("checkscam.vn")
                && (src.ends_with(".jpg") || src.ends_with(".jpeg")
                    || src.ends_with(".png") || src.ends_with(".webp"))
                && !src.contains("logo")
                && !src.contains("avatar")
                && !src.contains("icon")
        })
        .map(|s| s.to_owned())
        .collect();

    ExtractedEntities {
        phones: phones.into_iter().collect(),
        bank_accounts: bank_accounts.into_iter().collect(),
        owner,
        bank,
        warning,
        report_count,
        image_urls,
        source_url: post.link.clone(),
        title: post.title.rendered.clone(),
        date: Some(post.date.clone()),
    }
}
```

### Step 5: Implement image download

```rust
async fn download_image(
    client: &reqwest::Client,
    image_url: &str,
    media_dir: &str,
    subject_id: Uuid,
) -> anyhow::Result<(String, Option<String>, Option<i64>)> {
    let response = client.get(image_url).send().await?;
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    let bytes = response.bytes().await?;
    let file_size = bytes.len() as i64;

    // SHA256 hash for filename dedup
    let hash = hex::encode(Sha256::digest(&bytes));
    let ext = image_url
        .rsplit('.')
        .next()
        .filter(|e| ["jpg", "jpeg", "png", "webp"].contains(e))
        .unwrap_or("jpg");

    let relative_dir = format!("evidence/{subject_id}");
    let relative_path = format!("{relative_dir}/{hash}.{ext}");
    let full_dir = PathBuf::from(media_dir).join(&relative_dir);
    let full_path = PathBuf::from(media_dir).join(&relative_path);

    // Skip if already downloaded (same hash = same content)
    if full_path.exists() {
        return Ok((relative_path, content_type, Some(file_size)));
    }

    tokio::fs::create_dir_all(&full_dir).await?;
    tokio::fs::write(&full_path, &bytes).await?;

    Ok((relative_path, content_type, Some(file_size)))
}
```

**Note:** Add `hex` to Cargo.toml dependencies: `hex = "0.4"`

### Step 6: Implement `process_posts()`

```rust
async fn process_posts(
    kb: &KnowledgeBase,
    factory: &HttpClientFactory,
    posts: &[CrawlPostRecord],
    args: &Args,
) -> anyhow::Result<CrawlStats> {
    let mut stats = CrawlStats::default();
    let sidecar_available = check_sidecar_available().await;
    let sidecar_client = if sidecar_available {
        tracing::info!("sidecar available, will use for detail fetches");
        Some(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(35))
                .build()?,
        )
    } else {
        tracing::info!("sidecar unavailable, using REST content as fallback");
        None
    };

    let download_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let semaphore = Semaphore::new(args.concurrency);

    for (idx, post) in posts.iter().enumerate() {
        let _permit = semaphore.acquire().await?;

        // Resume check: skip if already processed
        if args.resume {
            let exists = evidence_exists_for_url(kb, &post.link).await;
            if exists {
                stats.skipped_existing += 1;
                if idx % 100 == 0 {
                    tracing::debug!(idx, total = posts.len(), url = %post.link, "skipped (exists)");
                }
                continue;
            }
        }

        tracing::info!(
            idx,
            total = posts.len(),
            url = %post.link,
            "processing post"
        );

        // Fetch detail HTML
        let detail_html = fetch_detail_html(
            sidecar_client.as_ref(),
            &post.content.rendered,
            &post.link,
        ).await;

        // Extract entities
        let entities = extract_entities(post, &detail_html);

        if entities.phones.is_empty() && entities.bank_accounts.is_empty() {
            tracing::debug!(url = %post.link, "no entities found, skipping");
            continue;
        }

        // Collect all mentioned values for cross-referencing
        let mut all_mentioned: Vec<String> = Vec::new();
        all_mentioned.extend(entities.phones.iter().cloned());
        all_mentioned.extend(entities.bank_accounts.iter().cloned());

        // Build risk signals
        let mut risk_signals = Vec::new();
        if entities.warning.is_some() {
            risk_signals.push("checkscam_warning".to_string());
        }
        if let Some(count) = entities.report_count {
            risk_signals.push(format!("checkscam_report_count:{count}"));
            if count >= 5 {
                risk_signals.push("multiple_reports_critical".to_string());
            } else if count >= 3 {
                risk_signals.push("multiple_reports_high".to_string());
            }
        }

        // Build evidence data JSON
        let evidence_data = serde_json::json!({
            "source_url": entities.source_url,
            "title": entities.title,
            "owner": entities.owner,
            "bank": entities.bank,
            "warning": entities.warning,
            "report_count": entities.report_count,
            "date": entities.date,
            "phones": entities.phones,
            "bank_accounts": entities.bank_accounts,
        });

        // Process each entity as a subject
        let mut process_subject = |value: &str, subject_type: &str| -> (String, String) {
            (value.to_owned(), subject_type.to_owned())
        };

        let mut subject_entries: Vec<(String, String)> = Vec::new();
        for phone in &entities.phones {
            subject_entries.push(process_subject(phone, "phone"));
        }
        for acct in &entities.bank_accounts {
            subject_entries.push(process_subject(acct, "bank"));
        }

        for (value, subject_type) in &subject_entries {
            // Upsert subject
            let subject_id = match kb.upsert_subject(value, subject_type).await {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!(value, subject_type, error = %e, "upsert_subject failed");
                    stats.errors += 1;
                    continue;
                }
            };
            stats.subjects_created += 1;

            // mentioned_subjects = all other entities in the same post
            let mentioned: Vec<String> = all_mentioned
                .iter()
                .filter(|v| v.as_str() != value)
                .cloned()
                .collect();

            // Insert evidence
            let evidence = EvidenceInput {
                subject_id,
                investigation_id: None,
                source: "checkscam_crawl".to_string(),
                evidence_type: "external_report".to_string(),
                data: evidence_data.clone(),
                risk_signals: risk_signals.clone(),
                mentioned_subjects: mentioned,
            };

            if let Err(e) = kb.insert_evidence_batch(vec![evidence]).await {
                tracing::warn!(url = %post.link, error = %e, "insert_evidence failed");
                stats.errors += 1;
                continue;
            }
            stats.evidence_inserted += 1;

            // Download images and insert media records
            for image_url in &entities.image_urls {
                // Dedup: skip if already downloaded
                match kb.media_exists_by_url(image_url).await {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(_) => continue,
                }

                match download_image(&download_client, image_url, &args.media_dir, subject_id).await {
                    Ok((file_path, content_type, file_size)) => {
                        if let Err(e) = kb
                            .insert_media(
                                "evidence",
                                subject_id,
                                &file_path,
                                Some(image_url),
                                content_type.as_deref(),
                                file_size,
                            )
                            .await
                        {
                            tracing::warn!(image_url, error = %e, "insert_media failed");
                        } else {
                            stats.images_downloaded += 1;
                        }
                    }
                    Err(e) => {
                        tracing::debug!(image_url, error = %e, "image download failed");
                    }
                }
            }

            // Set initial risk based on report_count (Phase 4 refines this)
            if let Some(count) = entities.report_count {
                let (risk_level, risk_score) = initial_risk_from_report_count(count);
                if let Err(e) = set_initial_risk(kb, subject_id, risk_level, risk_score).await {
                    tracing::warn!(subject_id = %subject_id, error = %e, "set_initial_risk failed");
                }
            }
        }

        // Rate limit between posts
        if args.delay_ms > 0 {
            sleep(Duration::from_millis(args.delay_ms)).await;
        }
    }

    Ok(stats)
}

async fn evidence_exists_for_url(kb: &KnowledgeBase, url: &str) -> bool {
    // Query evidence table for existing crawl data with this source URL
    let result = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
            SELECT 1 FROM evidence
            WHERE source = 'checkscam_crawl'
              AND data->>'source_url' = $1
        )",
    )
    .bind(url)
    .fetch_one(&kb.pool)
    .await;

    result.unwrap_or(false)
}

fn initial_risk_from_report_count(count: u32) -> (&'static str, f32) {
    match count {
        0..=2 => ("medium", 0.5),
        3..=4 => ("high", 0.7),
        _ => ("critical", 0.9),
    }
}

async fn set_initial_risk(
    kb: &KnowledgeBase,
    subject_id: Uuid,
    risk_level: &str,
    risk_score: f32,
) -> anyhow::Result<()> {
    // Only set risk if current risk is unknown/lower (don't downgrade)
    sqlx::query(
        "UPDATE subjects
         SET risk_score = GREATEST(risk_score, $2),
             risk_level = CASE
                 WHEN risk_score < $2 THEN $3
                 ELSE risk_level
             END,
             last_seen_at = NOW()
         WHERE id = $1",
    )
    .bind(subject_id)
    .bind(risk_score)
    .bind(risk_level)
    .execute(&kb.pool)
    .await?;
    Ok(())
}
```

### Step 7: Add missing dependencies to Cargo.toml

```toml
hex = "0.4"
```

`sha2` and `reqwest` already in Cargo.toml.

### Step 8: Compile and test

```bash
cargo check --bin checkscam-crawler
# Manual test with real DB and --max-pages 1
RUST_LOG=info cargo run --bin checkscam-crawler -- \
    --max-pages 1 --database-url "$DATABASE_URL"
```

## Todo List

- [ ] Add `ExtractedEntities` struct and helper types
- [ ] Implement `check_sidecar_available()`
- [ ] Implement `fetch_detail_html()` with sidecar + REST fallback
- [ ] Implement `extract_entities()` with regex extraction from HTML
- [ ] Implement `download_image()` with SHA256 hash naming
- [ ] Implement full `process_posts()` — upsert subjects, insert evidence, insert media
- [ ] Implement `evidence_exists_for_url()` for resume support
- [ ] Implement `initial_risk_from_report_count()` and `set_initial_risk()`
- [ ] Add `hex` dependency to Cargo.toml
- [ ] `cargo check --bin checkscam-crawler`
- [ ] Manual test: `--max-pages 1` processes posts, creates subjects + evidence + media

## Success Criteria

- Crawler processes posts end-to-end: fetch → parse → extract → download → DB insert
- Subjects created with correct types (phone/bank)
- Evidence records have `source="checkscam_crawl"`, `evidence_type="external_report"`, `investigation_id=NULL`
- Images downloaded to `data/media/evidence/{subject_id}/` with SHA256 naming
- Media records reference correct entity and file paths
- Resume mode skips already-processed posts
- No panics on malformed posts — errors logged and skipped

## Risk Assessment

- **Regex extraction accuracy:** Reusing proven patterns from existing scraper. Edge cases (unusual formatting) may miss some entities — acceptable for bulk seed data.
- **Image download failures:** Common (404, timeout). Logged and skipped — non-fatal.
- **Large post volume:** Thousands of posts with 3 concurrent fetches + 200ms delay = hours of runtime. Expected and acceptable for one-time seed crawl.
- **Pool field access:** `evidence_exists_for_url` and `set_initial_risk` access `kb.pool` directly. This field is `pub(crate)` — the binary is in the same crate, so this works. If it becomes an issue, add methods to KnowledgeBase.

## Security Considerations

- Downloaded images stored locally — no execution risk
- SHA256 naming prevents path traversal from malicious filenames
- No user input in SQL queries — all parameterized via sqlx bind

## Next Steps

Phase 4 adds link detection (mentioned_subjects → subject_links) and final summary output.
