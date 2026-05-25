# Debug Report: Google Search via Proxy Returns 0 Results

**Date:** 2026-05-12 15:02  
**Severity:** High — Google scraping completely non-functional through all available proxies  

---

## Executive Summary

Google returns a **JavaScript-execution challenge page** (`/httpservice/retry/enablejs`) to all proxy IPs tested. This page requires JS execution to set a verification cookie before redirecting to actual search results. Without JS execution, no search results HTML is ever served — hence 0 results. The previous codebase treated these pages as "not blocked" (`captcha: false`), masking the true failure mode.

---

## Root Cause

### Primary: JS Challenge Page Not Detected

`is_google_block_page()` did NOT check for `/httpservice/retry/enablejs`, so Google's JS challenge was classified as a successful (unblocked) response. Consequently:
- `successful_queries` was populated (query "succeeded")  
- `metadata.captcha = false` (looked like normal flow)
- But `parse_google_tch1()` → 0 JSON fragments (body is not JSON)
- And `parse_google_basic_results()` → 0 results (body has 0 `<h3>` tags, 0 class matches)

### Secondary: All Proxy IPs Trigger JS Challenge

Every proxy tested — ola (VN residential), MKVN (VN datacenter), proxiesthatwork (US datacenter), server (local SOCKS5 → traditional CAPTCHA) — triggers Google's JS challenge at the IP level. UA changes (GSA mobile → desktop Chrome) do not help; the challenge is IP-based bot detection.

**JS Challenge Page Anatomy:**
- Title: `"Bật JavaScript để sử dụng tính năng tìm kiếm"` (Enable JavaScript to use search)
- Body: pure obfuscated JS with TrustedTypes policy setup
- Mechanism: JS sets a cookie (`NID` or similar), then JS redirect to actual search
- Without JS execution: page loops at `/httpservice/retry/enablejs`, no search HTML ever served

### Tertiary: Wrong UA for Proxy Requests (Fixed, but not the root cause)

Original code used GSA mobile UAs (`NSTNWV` suffix) for all requests including proxied ones. These UAs request the `tch=1` JSON-fragment endpoint which requires a Google Search App session. Even if the JS challenge were bypassed, desktop UAs are more likely to get parseable HTML.

---

## Changes Made

### 1. `is_google_block_page()` — detect JS challenge (critical fix)
```rust
// Added to block page signals:
"/httpservice/retry/enablejs",
```
Effect: JS-challenged requests now correctly mark `captcha: true` and `queries_blocked`, so the pipeline knows to try the next proxy rather than treating 0 results as a successful empty search.

### 2. `HttpClientFactory::google_client()` — use desktop UAs
Replaced `GSA_USER_AGENTS` (Android mobile + NSTNWV) with `DESKTOP_USER_AGENTS` (Windows/Mac/Linux Chrome). Desktop UAs receive plain HTML search pages rather than JSON fragments, and are the standard target for HTML scraping.

### 3. `scrape_once()` — try plain HTML parse before second request
Added `parse_google_basic_results(&body)` as an intermediate step when `parse_google_tch1` yields nothing. With desktop UAs, the initial response body is already parseable HTML — no need for a second `gbv=1` request.

### 4. Diagnostic test added (permanent, `#[ignore]`)
`diagnoses_proxy_html_format_and_parser_mismatches` — fetches tch=1 + gbv=1 via specific proxy, dumps HTML previews, reports class name matches, fragment counts. Run with:
```
cargo test diagnoses_proxy_html_format -- --nocapture --ignored
```

---

## Test Results

All 16 unit tests pass. Existing parser tests unchanged.

**Live diagnostic before fix:**
- `tch1_is_blocked=false` (JS challenge not detected)
- `tch1_fragment_count=0, tch1_parsed_results=0`
- `metadata: captcha=false, queries_succeeded=[all 4 queries]`

**Live diagnostic after fix:**
- `tch1_is_blocked=true` (JS challenge correctly detected)
- All proxy attempts: `captcha=true, queries_blocked=[all 4 queries]`

---

## Remaining Problem (Not Fixed)

**All proxy IPs are being JS-challenged by Google.** The fix correctly classifies failures but does not resolve the underlying inability to get search results. To actually retrieve results, one of the following is required:

| Option | Complexity | Cost |
|--------|-----------|------|
| Google Custom Search API (100 free/day) | Low | $5/1k queries |
| SerpAPI / ScrapingBee | Low | $50+/mo |
| Headless Chrome with cookie pre-seeding | High | Infrastructure cost |
| Bright Data / Oxylabs residential proxies with Google-specific routing | Medium | $100+/mo |

---

## Unresolved Questions

1. **Are the ola/MKVN proxies supposed to support Google scraping?** Some proxy services explicitly block or don't route Google — check with provider.
2. **Does the VPN server proxy (192.168.1.x) return real results?** It shows traditional CAPTCHA (`/sorry/index`), not the JS challenge — meaning a different IP class. Worth testing if those VPN exit nodes have clean Google access.
3. **Is there a `pws=0&nfpr=1` or similar param combination** that bypasses the JS challenge on some proxy types? Not observed in testing.
4. **Should fallback to Google Custom Search API be added** for queries where all proxy attempts fail?
