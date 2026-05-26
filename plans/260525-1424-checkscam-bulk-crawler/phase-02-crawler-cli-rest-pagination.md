---
phase: 2
title: "Crawler CLI Binary + REST API Pagination"
status: completed
effort: 3h
depends_on: [phase-01]
---

# Phase 2: Crawler CLI Binary + REST API Pagination

## Context Links

- [Brainstorm report](../reports/brainstorm-260525-1424-checkscam-bulk-crawler.md)
- [Existing checkscam scraper](../../src/scrapers/checkscam.rs) — reuse `parse_detail_report`, REST helpers
- [HttpClientFactory](../../src/scrapers/http_client.rs)
- [AppConfig](../../src/config.rs)
- [Cargo.toml](../../Cargo.toml)

## Overview

Create standalone CLI binary (`cargo run --bin checkscam-crawler`) that connects to DB, paginates the WordPress REST API to collect all post URLs, then processes each post through a detail-fetch → parse → DB-insert pipeline.

## Key Insights

- WordPress REST API: `GET /wp-json/wp/v2/posts?per_page=100&page=N` returns JSON array. Empty array or 400 = no more pages. `X-WP-TotalPages` header gives total page count.
- Existing `RestPostRecord` struct in checkscam.rs is private — need to either make it `pub(crate)` or define a similar struct in the crawler. **Decision:** define a richer `CrawlPostRecord` in the crawler since we need more fields (id, content.rendered, date, link).
- `parse_detail_report()` is already a standalone function — just needs `pub(crate)` visibility.
- CLI args via `clap`: `--max-pages` (default unlimited), `--concurrency` (default 3), `--delay-ms` (default 200), `--dry-run`, `--resume` (skip posts already in DB).
- The crawler is a **separate binary** — it uses `tracuuluadao` as a library crate. This means `src/lib.rs` must expose the necessary modules.

## Requirements

**Functional:**
- CLI binary with configurable args (max-pages, concurrency, delay, dry-run, resume)
- Paginate WP REST API to collect all post metadata (link, title, content.rendered, date, WP post ID)
- Track progress with log output (page N/total, posts collected)
- Resume support: skip posts whose URL is already in evidence table

**Non-functional:**
- Rate limiting: configurable delay between page fetches (default 200ms)
- Graceful error handling: log and skip failed pages, continue crawling
- Must not interfere with running server

## Architecture

```
checkscam-crawler CLI
│
├── main() — parse args, init DB, run crawler
│
├── Phase A: Paginate REST API
│   ├── GET /wp-json/wp/v2/posts?per_page=100&page=1
│   ├── GET /wp-json/wp/v2/posts?per_page=100&page=2
│   ├── ... until empty response or max-pages reached
│   └── Collect Vec<CrawlPostRecord>
│
├── Phase B: Process posts (concurrent, rate-limited)
│   ├── For each post:
│   │   ├── fetch detail HTML (sidecar or REST content.rendered fallback)
│   │   ├── parse_detail_report() → extract entities
│   │   ├── extract + download images → data/media/evidence/
│   │   ├── upsert subjects, insert evidence, insert media
│   │   └── set initial risk
│   └── Semaphore(concurrency) + tokio::time::sleep(delay)
│
├── Phase C: Link detection pass (Phase 4)
│
└── Summary stats
```

## Related Code Files

**Create:**
- `src/lib.rs` — library crate root, re-exports modules for binary access
- `src/bin/checkscam_crawler.rs` — CLI entry point + crawler logic

**Modify:**
- `Cargo.toml` — add `[[bin]]` entry, add `clap` dependency
- `src/scrapers/checkscam.rs` — make `parse_detail_report` `pub(crate)`

## Implementation Steps

### Step 1: Create `src/lib.rs` (library crate root)

The binary needs access to library modules. Create `src/lib.rs` that re-exports necessary modules:

```rust
// src/lib.rs
pub mod config;
pub mod error;
pub mod knowledge_base;
pub mod scrapers;
```

**Important:** `main.rs` must switch from `mod` declarations to `use` imports for shared modules. However, since `main.rs` also uses private modules (`agents`, `api`, `cache`, `logging`, `pipeline`, `report_store`), we have two options:

**Option A (recommended):** Keep `main.rs` with its own `mod` declarations for private modules, and `use tracuuluadao::*` for shared ones. But this conflicts — Rust doesn't allow a module declared in both lib.rs and main.rs.

**Option B (simpler):** Don't create lib.rs. Instead, the binary directly depends on the modules via path. But Cargo doesn't support this for binaries in the same crate without lib.rs.

**Decision: Option A with restructure.** Move ALL mod declarations to `lib.rs`, make `main.rs` use the library:

```rust
// src/lib.rs
pub mod agents;
pub mod api;
pub mod cache;
pub mod config;
pub mod error;
pub mod knowledge_base;
pub mod logging;
pub mod pipeline;
pub mod report_store;
pub mod scrapers;
```

```rust
// src/main.rs (updated)
use std::net::SocketAddr;
use std::sync::Arc;

use tracuuluadao::api;
use tracuuluadao::config::AppConfig;
use tracuuluadao::error::AppResult;
use tracuuluadao::logging;

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();
    let config = Arc::new(AppConfig::from_env()?);
    let _log_guards = logging::init()?;
    let state = api::build_state(config.clone()).await?;
    let app = api::router(state);

    let listener =
        tokio::net::TcpListener::bind((config.app_host.as_str(), config.app_port)).await?;
    tracing::info!("listening on {}:{}", config.app_host, config.app_port);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
```

**Note:** Some modules may have `pub(crate)` visibility that needs to become `pub`. Check each module and adjust visibility as needed during implementation. Only the modules/items actually used by the crawler binary need `pub` — the rest can stay `pub(crate)`.

**Critical visibility fix:** `KnowledgeBase.pool` is `pub(crate)` in `src/knowledge_base/mod.rs`. The binary (separate crate) **cannot** access `pub(crate)` items. Do NOT change `pool` to `pub`. Instead, add dedicated methods to `KnowledgeBase` in Phase 3 (e.g., `evidence_exists_for_url`, `set_crawl_risk`) that encapsulate pool access. This keeps the pool private and maintains proper encapsulation.

### Step 2: Add clap dependency and `[[bin]]` to Cargo.toml

```toml
# Add to [dependencies]
clap = { version = "4", features = ["derive"] }

# Add at bottom of Cargo.toml
[[bin]]
name = "checkscam-crawler"
path = "src/bin/checkscam_crawler.rs"
```

### Step 3: Make `parse_detail_report` pub(crate)

In `src/scrapers/checkscam.rs`, change:

```rust
// FROM:
fn parse_detail_report(url: &str, fallback_title: &str, html: &str) -> ScrapedReport {
// TO:
pub fn parse_detail_report(url: &str, fallback_title: &str, html: &str) -> ScrapedReport {
```

Also make `SIDECAR_BASE_URL` pub:

```rust
pub const SIDECAR_BASE_URL: &str = "http://127.0.0.1:4417";
```

### Step 4: Create the crawler binary

`src/bin/checkscam_crawler.rs`:

```rust
use std::time::Duration;

use clap::Parser;
use sqlx::PgPool;
use tokio::time::sleep;
use tracing_subscriber::EnvFilter;

use tracuuluadao::knowledge_base::KnowledgeBase;
use tracuuluadao::scrapers::checkscam;
use tracuuluadao::scrapers::http_client::HttpClientFactory;

/// Bulk-crawl checkscam.vn posts into the knowledge base.
#[derive(Parser, Debug)]
#[command(name = "checkscam-crawler")]
struct Args {
    /// Maximum number of REST API pages to fetch (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    max_pages: usize,

    /// Number of concurrent detail fetches
    #[arg(long, default_value_t = 3)]
    concurrency: usize,

    /// Delay in milliseconds between REST API page fetches
    #[arg(long, default_value_t = 200)]
    delay_ms: u64,

    /// Print what would be done without writing to DB or disk
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Skip posts whose URL already exists in evidence table
    #[arg(long, default_value_t = true)]
    resume: bool,

    /// Override DATABASE_URL
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Base directory for media storage (default: data/media)
    #[arg(long, default_value = "data/media")]
    media_dir: String,
}

const REST_API_BASE: &str = "https://checkscam.vn/wp-json/wp/v2/posts";
const REST_PAGE_SIZE: usize = 100;

/// Represents a post from the WP REST API with fields needed for crawling.
#[derive(Debug, serde::Deserialize)]
struct CrawlPostRecord {
    id: u64,
    link: String,
    title: WpRenderedField,
    content: WpRenderedField,
    date: String,
}

#[derive(Debug, serde::Deserialize)]
struct WpRenderedField {
    rendered: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    tracing::info!(?args, "starting checkscam crawler");

    // 1. Connect to DB
    let pool = PgPool::connect(&args.database_url).await?;
    let kb = KnowledgeBase::new(pool);
    kb.ensure_schema().await
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    // 2. HTTP client for REST API
    let factory = HttpClientFactory::default();
    let client = factory.standard_client()
        .map_err(|e| anyhow::anyhow!("http client failed: {e}"))?;

    // 3. Paginate REST API
    let posts = paginate_rest_api(&client, &args).await?;
    tracing::info!(total_posts = posts.len(), "REST API pagination complete");

    if args.dry_run {
        for post in &posts {
            tracing::info!(
                id = post.id,
                link = %post.link,
                title = %post.title.rendered,
                "DRY RUN: would process"
            );
        }
        tracing::info!(total = posts.len(), "dry run complete");
        return Ok(());
    }

    // 4. Process posts (Phase 3 handles detail parsing + image download + DB insert)
    let stats = process_posts(&kb, &factory, &posts, &args).await?;

    // 5. Link detection pass (Phase 4)
    // link_detection_pass(&kb).await?;

    // 6. Summary
    tracing::info!(
        total_posts = posts.len(),
        subjects_created = stats.subjects_created,
        evidence_inserted = stats.evidence_inserted,
        images_downloaded = stats.images_downloaded,
        skipped_existing = stats.skipped_existing,
        errors = stats.errors,
        "crawl complete"
    );

    Ok(())
}

async fn paginate_rest_api(
    client: &tracuuluadao::scrapers::http_client::HttpClient,
    args: &Args,
) -> anyhow::Result<Vec<CrawlPostRecord>> {
    let mut all_posts = Vec::new();
    let mut page = 1u64;

    loop {
        if args.max_pages > 0 && page as usize > args.max_pages {
            tracing::info!(page, max_pages = args.max_pages, "reached max pages limit");
            break;
        }

        let url = url::Url::parse_with_params(REST_API_BASE, &[
            ("per_page", REST_PAGE_SIZE.to_string()),
            ("page", page.to_string()),
            ("orderby", "date".to_string()),
            ("order", "desc".to_string()),
        ])?;

        tracing::info!(page, url = %url, "fetching REST API page");

        let body = match client.get_text_from_url(url).await {
            Ok(body) => body,
            Err(error) => {
                // WP returns 400 when page is beyond total — treat as end
                tracing::info!(page, error = %error, "REST API page fetch failed, assuming end");
                break;
            }
        };

        let page_posts: Vec<CrawlPostRecord> = match serde_json::from_str(&body) {
            Ok(posts) => posts,
            Err(error) => {
                tracing::warn!(page, error = %error, "failed to parse REST response");
                break;
            }
        };

        if page_posts.is_empty() {
            tracing::info!(page, "empty page, pagination complete");
            break;
        }

        let count = page_posts.len();
        all_posts.extend(page_posts);
        tracing::info!(page, posts_on_page = count, total_so_far = all_posts.len(), "page collected");

        if count < REST_PAGE_SIZE {
            break;
        }

        page += 1;
        sleep(Duration::from_millis(args.delay_ms)).await;
    }

    Ok(all_posts)
}

/// Crawl stats for summary output.
#[derive(Debug, Default)]
struct CrawlStats {
    subjects_created: usize,
    evidence_inserted: usize,
    images_downloaded: usize,
    skipped_existing: usize,
    errors: usize,
}

/// Placeholder — full implementation in Phase 3.
async fn process_posts(
    _kb: &KnowledgeBase,
    _factory: &HttpClientFactory,
    _posts: &[CrawlPostRecord],
    _args: &Args,
) -> anyhow::Result<CrawlStats> {
    // Phase 3 fills this in
    Ok(CrawlStats::default())
}
```

### Step 5: Verify compilation

```bash
cargo check --bin checkscam-crawler
cargo check  # ensure main binary still compiles
```

### Step 6: Test REST pagination (manual)

```bash
# Dry run — just paginate and log, no DB writes
RUST_LOG=info cargo run --bin checkscam-crawler -- --dry-run --max-pages 2 --database-url "$DATABASE_URL"
```

## Todo List

- [x] Create `src/lib.rs` with module re-exports
- [x] Update `src/main.rs` to use library imports instead of `mod` declarations
- [x] Add `clap` to `[dependencies]` in Cargo.toml
- [x] Add `[[bin]]` entry for `checkscam-crawler` in Cargo.toml
- [x] Make `parse_detail_report` and `SIDECAR_BASE_URL` pub in checkscam.rs
- [x] Create `src/bin/checkscam_crawler.rs` with CLI args + REST pagination
- [x] `cargo check` — both binaries compile
- [x] Manual test: `--dry-run --max-pages 2` successfully paginates

## Success Criteria

- [x] `cargo check --bin checkscam-crawler` compiles
- [x] `cargo check` (default binary) still compiles
- [x] Dry-run mode paginates REST API and logs post URLs without DB writes
- [x] CLI args (--max-pages, --concurrency, --delay-ms, --dry-run, --resume) work correctly

## Risk Assessment

- **lib.rs restructure:** Main risk. Moving from `mod` to library crate may surface visibility issues. Mitigation: adjust `pub(crate)` → `pub` only where needed by the crawler binary. Keep non-shared modules private.
- **REST API blocks:** Low risk — WP REST API rarely behind Cloudflare. If blocked, can route through sidecar.
- **Large pagination:** checkscam may have thousands of posts. Memory usage: ~1KB per CrawlPostRecord × 10K posts = ~10MB. Acceptable.

## Security Considerations

- DATABASE_URL from env or CLI arg — never hardcoded
- No secrets in binary output (logs only show URLs and counts)

## Next Steps

Phase 3 implements `process_posts()` — the core detail-fetch, parse, image-download, and DB-insert logic.
