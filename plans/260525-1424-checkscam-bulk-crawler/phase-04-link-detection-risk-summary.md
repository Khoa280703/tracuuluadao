---
phase: 4
title: "Link Detection + Risk Assignment + Summary"
status: pending
effort: 1.5h
depends_on: [phase-03]
---

# Phase 4: Link Detection + Risk Assignment + Summary

## Context Links

- [Phase 3 — Detail parsing + DB insert](./phase-03-detail-parsing-image-db.md)
- [upsert_subject_link()](../../src/knowledge_base/subjects.rs) — lines 115-151
- [recalculate_risk()](../../src/knowledge_base/risk.rs) — lines 43-89
- [Brainstorm — link detection](../reports/brainstorm-260525-1424-checkscam-bulk-crawler.md)

## Overview

After all posts are processed, run a second pass to build `subject_links` from `mentioned_subjects` stored in evidence records. Then recalculate risk scores for all affected subjects. Print final crawl summary.

## Key Insights

- `mentioned_subjects` is already populated in Phase 3 — each evidence record lists other entities found in the same post
- Link detection is a post-processing step, not per-post — avoids duplicate link inserts and runs faster as a batch
- `upsert_subject_link()` already handles dedup (ON CONFLICT strength += 1)
- Risk recalculation uses the existing `recalculate_risk()` which weighs investigations + approved reports. For crawled data with no investigations, the Phase 3 `set_initial_risk()` provides baseline. We run `recalculate_risk()` after link detection to catch any subjects that gained report evidence from user_reports table.
- Keep it simple: link_type = "co_mentioned" for all links from same-post co-occurrence

## Requirements

**Functional:**
- Query all evidence with source="checkscam_crawl" that has non-empty mentioned_subjects
- For each evidence record: resolve mentioned_subjects (text values) to subject IDs
- Create subject_links between the evidence's subject and each mentioned subject
- Recalculate risk for all subjects touched by the crawl
- Print final summary with counts

**Non-functional:**
- Batch processing — no per-post DB round trips during link pass
- Idempotent — `upsert_subject_link` handles re-runs gracefully (increases strength)

## Architecture

```
link_detection_pass()
├── Query: SELECT id, subject_id, mentioned_subjects
│          FROM evidence WHERE source = 'checkscam_crawl'
│          AND mentioned_subjects != '{}'
│
├── For each evidence record:
│   ├── For each mentioned value:
│   │   ├── kb.get_subject_by_value(value, inferred_type) → Option<subject_id>
│   │   └── If found: kb.upsert_subject_link(evidence.subject_id, mentioned_id, "co_mentioned", evidence.id)
│   └── Collect all touched subject IDs
│
├── Deduplicate touched subject IDs
├── For each subject_id:
│   └── kb.recalculate_risk(subject_id)
│
└── Return link stats
```

## Related Code Files

**Modify:**
- `src/bin/checkscam_crawler.rs` — implement `link_detection_pass()`, uncomment call in main, add final summary

**Read:**
- `src/knowledge_base/subjects.rs` — `upsert_subject_link`, `get_subject_by_value`
- `src/knowledge_base/risk.rs` — `recalculate_risk`

## Implementation Steps

### Step 1: Add evidence query struct

Add to `checkscam_crawler.rs`:

```rust
/// Minimal evidence record for link detection pass.
#[derive(Debug, sqlx::FromRow)]
struct EvidenceForLinking {
    id: Uuid,
    subject_id: Uuid,
    mentioned_subjects: Vec<String>,
}
```

### Step 2: Implement link detection pass

```rust
async fn link_detection_pass(kb: &KnowledgeBase) -> anyhow::Result<LinkStats> {
    tracing::info!("starting link detection pass");
    let mut stats = LinkStats::default();

    // Fetch all crawled evidence with mentioned_subjects
    let evidence_records = sqlx::query_as::<_, EvidenceForLinking>(
        "SELECT id, subject_id, mentioned_subjects
         FROM evidence
         WHERE source = 'checkscam_crawl'
           AND array_length(mentioned_subjects, 1) > 0",
    )
    .fetch_all(&kb.pool)
    .await?;

    tracing::info!(
        records = evidence_records.len(),
        "evidence records with mentions found"
    );

    let mut touched_subjects: HashSet<Uuid> = HashSet::new();

    for record in &evidence_records {
        touched_subjects.insert(record.subject_id);

        for mentioned_value in &record.mentioned_subjects {
            // Infer subject type from the value
            let subject_type = infer_subject_type(mentioned_value);

            // Look up the mentioned subject
            let mentioned_subject = match kb
                .get_subject_by_value(mentioned_value, subject_type)
                .await
            {
                Ok(Some(subject)) => subject,
                Ok(None) => {
                    // Subject not in DB — might be an entity we didn't upsert
                    // (e.g., owner name). Skip.
                    continue;
                }
                Err(e) => {
                    tracing::debug!(
                        value = mentioned_value,
                        error = %e,
                        "get_subject_by_value failed"
                    );
                    continue;
                }
            };

            // Don't link a subject to itself
            if mentioned_subject.id == record.subject_id {
                continue;
            }

            // Create link
            if let Err(e) = kb
                .upsert_subject_link(
                    record.subject_id,
                    mentioned_subject.id,
                    "co_mentioned",
                    Some(record.id),
                )
                .await
            {
                tracing::debug!(
                    a = %record.subject_id,
                    b = %mentioned_subject.id,
                    error = %e,
                    "upsert_subject_link failed"
                );
                stats.link_errors += 1;
                continue;
            }

            touched_subjects.insert(mentioned_subject.id);
            stats.links_created += 1;
        }
    }

    // Recalculate risk for all touched subjects
    tracing::info!(
        subjects = touched_subjects.len(),
        "recalculating risk for touched subjects"
    );
    for subject_id in &touched_subjects {
        if let Err(e) = kb.recalculate_risk(*subject_id).await {
            tracing::debug!(subject_id = %subject_id, error = %e, "recalculate_risk failed");
            stats.risk_errors += 1;
        }
    }
    stats.subjects_risk_updated = touched_subjects.len();

    tracing::info!(?stats, "link detection pass complete");
    Ok(stats)
}

/// Infer subject_type from a raw value string.
/// Phone numbers are digits starting with 0, bank accounts are longer digit strings.
fn infer_subject_type(value: &str) -> &'static str {
    let cleaned: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    if cleaned.len() >= 9 && cleaned.starts_with('0') {
        "phone"
    } else if cleaned.len() >= 6 {
        "bank"
    } else {
        "phone" // default fallback
    }
}

#[derive(Debug, Default)]
struct LinkStats {
    links_created: usize,
    link_errors: usize,
    subjects_risk_updated: usize,
    risk_errors: usize,
}
```

### Step 3: Uncomment link detection call in main()

In the `main()` function, replace the commented-out line:

```rust
// FROM:
// link_detection_pass(&kb).await?;

// TO:
let link_stats = link_detection_pass(&kb).await?;
```

### Step 4: Update final summary output

Replace the summary block in `main()`:

```rust
tracing::info!(
    total_posts = posts.len(),
    subjects_created = stats.subjects_created,
    evidence_inserted = stats.evidence_inserted,
    images_downloaded = stats.images_downloaded,
    skipped_existing = stats.skipped_existing,
    processing_errors = stats.errors,
    links_created = link_stats.links_created,
    subjects_risk_updated = link_stats.subjects_risk_updated,
    "crawl complete"
);

// Also print human-readable summary to stdout
println!("\n=== Checkscam Crawl Summary ===");
println!("Posts fetched:        {}", posts.len());
println!("Subjects created:     {}", stats.subjects_created);
println!("Evidence inserted:    {}", stats.evidence_inserted);
println!("Images downloaded:    {}", stats.images_downloaded);
println!("Skipped (existing):   {}", stats.skipped_existing);
println!("Links created:        {}", link_stats.links_created);
println!("Risk updated:         {}", link_stats.subjects_risk_updated);
println!("Errors:               {}", stats.errors + link_stats.link_errors + link_stats.risk_errors);
```

### Step 5: Add --skip-links CLI flag

Add to `Args`:

```rust
/// Skip link detection pass (faster for testing)
#[arg(long, default_value_t = false)]
skip_links: bool,
```

Update main():

```rust
let link_stats = if args.skip_links {
    tracing::info!("skipping link detection pass (--skip-links)");
    LinkStats::default()
} else {
    link_detection_pass(&kb).await?
};
```

### Step 6: Compile and end-to-end test

```bash
cargo check --bin checkscam-crawler

# Full crawl test (small batch)
RUST_LOG=info cargo run --bin checkscam-crawler -- \
    --max-pages 1 --database-url "$DATABASE_URL"

# Verify data in DB
psql "$DATABASE_URL" -c "
    SELECT subject_type, COUNT(*) FROM subjects GROUP BY subject_type;
    SELECT source, COUNT(*) FROM evidence GROUP BY source;
    SELECT entity_type, COUNT(*) FROM media GROUP BY entity_type;
    SELECT link_type, COUNT(*) FROM subject_links GROUP BY link_type;
"

# Test resume (re-run should skip all)
RUST_LOG=info cargo run --bin checkscam-crawler -- \
    --max-pages 1 --database-url "$DATABASE_URL"
# Expect: skipped_existing = N, subjects_created = 0
```

## Todo List

- [ ] Add `EvidenceForLinking` struct
- [ ] Implement `link_detection_pass()` — query evidence, resolve mentions, create links
- [ ] Implement `infer_subject_type()` helper
- [ ] Add `LinkStats` struct
- [ ] Uncomment and wire `link_detection_pass()` in `main()`
- [ ] Update summary output (structured log + human-readable stdout)
- [ ] Add `--skip-links` CLI flag
- [ ] `cargo check --bin checkscam-crawler`
- [ ] End-to-end test: `--max-pages 1` creates subjects, evidence, media, links
- [ ] Verify resume mode: re-run skips existing posts
- [ ] Verify DB data integrity with psql queries

## Success Criteria

- `subject_links` created between co-mentioned entities from same post
- Risk scores recalculated for all subjects touched by crawl
- `--skip-links` flag works for faster test iterations
- Re-running crawler is idempotent — no duplicate subjects, evidence, or links (strength increases on links)
- Human-readable summary printed to stdout
- `cargo check` passes for both binaries

## Risk Assessment

- **Link volume:** Each post with N entities creates N*(N-1)/2 links. A post with 3 entities = 3 links. Thousands of posts = potentially tens of thousands of links. `upsert_subject_link` handles this via ON CONFLICT. Performance acceptable.
- **Infer subject_type accuracy:** Simple heuristic (digit count + leading 0). May misclassify some edge cases. Acceptable for bulk seed — manual review can correct.
- **Risk recalculation cost:** One DB query per subject. Thousands of subjects = thousands of queries. Sequential but fast (simple SQL). Could batch in future if needed.

## Security Considerations

- No new attack surface — CLI binary only runs locally
- All SQL parameterized via sqlx bind

## Next Steps

After all 4 phases:
1. Run full crawl against checkscam.vn
2. Verify data quality with sample checks
3. Test that investigation pipeline finds crawled subjects (pre-query enrichment)
4. Consider adding progress bar (`indicatif` crate) for large crawls — optional enhancement
