# Phase 4: Scraping Module

## Priority: Critical | Effort: L | Status: completed

## Overview

Implement all 5 data source scrapers in Rust using `rquest` (TLS impersonation) + `scraper` (HTML parsing). Parallel execution with proxy rotation for Google.

## Current Reality

- All planned scraper modules exist and `run_all_scrapers()` runs them in parallel with Google → DuckDuckGo fallback.
- `src/scrapers/http_client.rs` now switches to real `rquest` impersonation behind `tls-impersonation`; the same wrapper is reused by deep URL fetches.
- Google parser now extracts snippets from `tch=1` fragments and has regression tests for snippet extraction and URL normalization.
- Live validation shows the scraper phase completes on real queries and scrape-cache refresh can be targeted per source.

## Context

- **Research report:** `plans/reports/research-260509-2046-google-scraping-strategy.md`
- **Python prototype:** `scripts/test-scrape-all-sources.py` (reference implementation)
- **Proxy files:** `proxies/` directory

## Requirements

- 5 scrapers: checkscam.vn, chongluadao.vn, trangtrang.com, tinnhiemmang.vn, Google/DDG
- TLS impersonation (Chrome fingerprint) for Cloudflare bypass
- Proxy rotation for Google (GSA UA + tch=1 trick)
- DuckDuckGo fallback when Google fails
- All scrapers return unified `ScrapedResult` type

## Architecture

```
src/scrapers/
├── mod.rs              # ScrapedResult type, run_all_scrapers()
├── checkscam.rs        # checkscam.vn scraper
├── chongluadao.rs      # chongluadao.vn scraper
├── trangtrang.rs       # trangtrang.com scraper
├── tinnhiemmang.rs     # tinnhiemmang.vn scraper
├── google.rs           # Google Search (GSA UA + tch=1 + proxy)
├── duckduckgo.rs       # DuckDuckGo HTML fallback
├── proxy.rs            # Proxy pool loader + rotation
└── http_client.rs      # Shared rquest client with TLS impersonation
```

### Unified Result Type

```rust
pub struct ScrapedResult {
    pub source: SourceName,
    pub query: String,
    pub reports: Vec<ScrapedReport>,     // For scam sites
    pub search_results: Vec<SearchResult>, // For Google/DDG
    pub raw_html: Option<String>,        // Fallback if parsing fails
    pub duration_ms: u64,
    pub error: Option<String>,
}

pub struct ScrapedReport {
    pub title: String,
    pub url: String,
    pub content: String,       // Full report text (for LLM to summarize)
    pub date: Option<String>,
}

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}
```

## Implementation Steps

### 1. Shared HTTP Client (`http_client.rs`)
- Create `rquest::Client` with Chrome TLS impersonation
- GSA User-Agent pool (rotate for Google)
- Standard Chrome UA for other sites
- Connection pooling, timeout config

### 2. Proxy Pool (`proxy.rs`)
- Load proxies from markdown files in `proxies/` dir
- Parse formats: `ip:port:user:pass` → `http://user:pass@ip:port`
- Parse formats: `socks5h://user:pass@ip:port`
- Random rotation: `get_random_proxy() -> Option<Proxy>`

### 3. checkscam.vn (`checkscam.rs`)
- GET `https://checkscam.vn/?qh_ss={query}`
- Parse: report count (regex `Có\s+(\d+)\s+cảnh báo`)
- Extract report slugs from links (filter nav/static pages via STATIC_SLUGS set)
- Fetch each report page `https://checkscam.vn/{slug}` → extract full content
- Return: list of ScrapedReport

### 4. chongluadao.vn (`chongluadao.rs`)
- **Phone lookup:** GET `https://feeds.chongluadao.vn/checkphone?q={query}` → JSON array
- **Report check:** GET `https://feeds.chongluadao.vn/reports/check-exists?q={query}` → JSON
- Both return JSON directly, no HTML parsing needed
- Combine results from both endpoints

### 5. trangtrang.com (`trangtrang.rs`)
- GET `https://trangtrang.com/{query}`
- Parse HTML: report section, comments, risk indicators

### 6. tinnhiemmang.vn (`tinnhiemmang.rs`)
- **Step 1:** GET `https://tinnhiemmang.vn` → extract XSRF-TOKEN from cookies
- **Step 2:** POST `https://tinnhiemmang.vn/searchOrg` with:
  - Body: `search={query}` (form-urlencoded)
  - Headers: `X-XSRF-TOKEN: {token}`, `Referer: https://tinnhiemmang.vn/`
- Parse HTML response: trust score, reports, domain info

### 7. Google Search (`google.rs`)
- GSA UA + `tch=1` parameter
- URL: `https://www.google.com/search?q={phone}+lừa+đảo&hl=vi&gl=vn&tch=1`
- Parse concatenated JSON objects (raw_decode loop)
- Extract h3 titles + URLs from HTML fragments
- On fail → retry with different proxy → return error for DDG fallback

### 8. DuckDuckGo (`duckduckgo.rs`)
- POST `https://html.duckduckgo.com/html/` with `q={phone}+lừa+đảo`
- Parse `.result` elements: title, URL, snippet
- No proxy needed

### 9. Orchestrator (`mod.rs`)
- `run_all_scrapers(query, proxy_pool) -> Vec<ScrapedResult>`
- Spawn all 5 scrapers in parallel via `tokio::join!`
- Google failure → trigger DDG fallback
- Individual scraper timeout: 10s each
- Return all results (including partial failures)

## Key Code Snippet — Google tch=1 Parser

```rust
fn parse_google_tch1(body: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut pos = 0;
    let bytes = body.as_bytes();
    
    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        if pos >= bytes.len() { break; }
        
        // Try parse JSON object
        match serde_json::from_str::<serde_json::Value>(&body[pos..]) {
            // Use streaming deserializer to handle concatenated objects
            // Extract "d" field containing HTML fragment
        }
    }
    
    // Parse all "d" HTML fragments with scraper crate
    // Extract h3 + parent <a> href
    results
}
```

## Success Criteria

- [x] Concrete scraper modules exist for all planned sources plus DDG fallback
- [x] Proxy pool loads from `proxies/` and Google fallback path is implemented
- [x] Each scraper path is exercised against real test queries in this pass
- [x] Google parser quality is covered by regression tests for snippets and normalized URLs
- [x] `cargo check --features tls-impersonation` passes and app boots with the feature enabled
- [x] Live validation proves targeted scrape refresh works with per-source cache keys

## Risk Assessment

- **rquest vs primp** — rquest is more maintained (check latest crate status), fallback to reqwest if needed
- **checkscam.vn HTML structure changes** — Parser may break, need monitoring
- **Google tch=1 format instability** — Fallback to regular HTML parsing mode
- **Proxy pool exhaustion** — If all proxies blocked, DDG fallback covers
