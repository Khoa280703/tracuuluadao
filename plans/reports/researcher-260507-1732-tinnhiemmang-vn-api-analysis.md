# API Research Report: tinnhiemmang.vn

**Date:** 2026-05-07  
**Status:** WORKING ENDPOINTS IDENTIFIED

---

## Executive Summary

tinnhiemmang.vn is a Laravel-based Vietnamese trust/reputation lookup platform with Cloudflare protection. Successfully bypassed with curl_cffi (chrome124 fingerprint). Identified working API endpoint for organization search. Domain/website lookup appears to be UI-only (no dedicated API found).

---

## Technical Stack

| Property | Value |
|----------|-------|
| Framework | Laravel |
| Protection | Cloudflare |
| Frontend | jQuery, Bootstrap |
| CSRF | X-CSRF-TOKEN (meta tag) |
| Cookie Auth | XSRF-TOKEN, tinnhiemmang_session |

---

## Working Endpoints

### 1. Organization Search (CONFIRMED WORKING)

**Endpoint:** `POST /searchOrg`

**Parameters:**
- `q` (string): Search query for organization name

**Example:**
```bash
curl -X POST https://tinnhiemmang.vn/searchOrg \
  -H "X-CSRF-TOKEN: {csrf_token}" \
  -H "X-Requested-With: XMLHttpRequest" \
  -d "q=shopee"
```

**Response:** HTML list of matching organizations with:
- Organization name
- Logo URL (`/storage/photos/shares/uploads/...`)
- Organization ID
- Logo path

**Tested Queries:**
- "shopee" → Returns Shopee (org_id: 36)
- "bank" → Returns multiple banks (4+ results)
- "scam" → Returns datascam.vn (org_id: 7532)

**Response Format Example:**
```html
<ul>
  <li>
    <div class="thumb">
      <img src="https://tinnhiemmang.vn/storage/photos/shares/uploads/shopee-9837.png" alt="logo">
    </div>
    <span class="webkit-box-1">Shopee</span>
    <input type="hidden" name="org_id" value="36">
    <input type="hidden" name="org_logo" value="https://...">
  </li>
</ul>
```

**Authentication:** Requires valid XSRF-TOKEN (from /meta[name="csrf-token"]) + session cookies

---

## Non-Working / Unreachable Endpoints

### Tested but Non-Functional

| Endpoint | Status | Notes |
|----------|--------|-------|
| `/api/search` | 200 (homepage) | Returns homepage, not API |
| `/api/check` | 200 (homepage) | Returns homepage |
| `/api/domain` | 200 (homepage) | Returns homepage |
| `/tim-kiem-theo-ten?q=...` | 500 | Server error |
| `/filterObj?name_obj=...` | 500 | Server error |
| `/check-domain/...` | 200 (homepage) | Returns homepage |
| `/domain/...` | 200 (homepage) | Returns homepage |
| `/graphql` | 302 redirect | Not implemented |
| `/wp-json/*` | 302 redirect | Not WordPress |
| `/xmlrpc.php` | 302 redirect | Not XML-RPC |

---

## Page Routes (UI-Only)

| Route | Purpose | Search Capability |
|-------|---------|-------------------|
| `/` | Homepage | Form-based search via JS |
| `/website-tin-nhiem` | Website trust listings | No working API found |
| `/he-thong-tin-nhiem` | System trust listings | No working API found |
| `/tim-kiem` | Search results (broken) | Returns homepage |
| `/registerTrust` | Register website/org | Form-based (POST) |

---

## Homepage Form Analysis

**Search Form:**
- **Action:** `/tim-kiem-theo-ten` (GET)
- **Method:** GET
- **Parameters:**
  - `q` (text) - Search query
  - `type` (hidden) - Unknown purpose (value not visible in form)

**Status:** Form is broken (500 error on submission)

---

## Session & Authentication

### Cookie Requirements
1. `XSRF-TOKEN` - Encrypted CSRF token (Laravel)
2. `tinnhiemmang_session` - Session identifier

### Headers for API Requests
```
X-CSRF-TOKEN: {value from meta[name="csrf-token"]}
X-Requested-With: XMLHttpRequest
Content-Type: application/x-www-form-urlencoded
```

### Token Extraction
```html
<meta name="csrf-token" content="...">
```

---

## Frontend Analysis

**Key JS Files:**
- `/js/theme1.js` (378KB) - Minified, contains UI logic

**Scripts on Homepage:**
- 14 inline + external script tags
- Google Analytics (GA)
- Facebook SDK
- Cloudflare Insights
- Bootstrap modals for registration

**JavaScript Patterns:**
- jQuery-based AJAX
- Form submission handlers
- Organization search autocomplete (uses `/searchOrg`)

---

## Limitations & Findings

### No Public JSON API
- tinnhiemmang.vn has NO public JSON REST API
- All endpoints return HTML (except static files)
- API calls are for internal UI functionality only

### Search Functionality is Broken
- `/tim-kiem-theo-ten` endpoint returns 500 error
- Form-based search on homepage does not work
- Only `/searchOrg` (organization autocomplete) is functional

### Database-Driven Content
- Organizations stored in database with IDs
- Logo assets in `/storage/photos/shares/uploads/`
- No direct database query endpoints exposed

---

## Data Points Available Through Working Endpoints

Via `/searchOrg`:
1. Organization name
2. Organization ID
3. Logo URL (public accessible)
4. Organization type (inferred from list)

**Not Available:**
- Trust ratings
- Verification status
- Historical data
- Domain associations
- Contact information

---

## Recommendations for Integration

### Option 1: Scrape /searchOrg Results (Simple)
- Use `/searchOrg` to identify organizations
- Map org_id to potential domain/website
- Limited usefulness (org search only, no domain lookup)

### Option 2: Web Scraping (More Complex)
- Scrape `/website-tin-nhiem` list pages
- Parse HTML for trust information
- Handle pagination if exists
- Risk: Terms of Service violation

### Option 3: Reverse-Engineer Frontend (Not Recommended)
- Analyze theme1.js for hidden endpoints
- Test various parameter combinations
- High fragility risk if site updates

---

## Browser TLS Fingerprinting

**Bypass Method:** curl_cffi with `impersonate="chrome124"`

**Why:** Cloudflare JAChallenge requires valid TLS fingerprint

**Dependencies:**
```bash
pip install --user --break-system-packages curl_cffi
```

**Usage:**
```python
from curl_cffi import requests as cf_requests
r = cf_requests.get(
    "https://tinnhiemmang.vn/searchOrg",
    params={"q": "shopee"},
    impersonate="chrome124",
    timeout=10
)
```

---

## Unresolved Questions

1. **Domain/Website Lookup:** Is there a separate API for checking specific domains? (No endpoint found yet)
2. **Trust Data Retrieval:** How does the frontend display trust ratings without an API? (Possibly embedded in HTML or via hidden XHR)
3. **Rate Limiting:** Are there rate limits on `/searchOrg`? (Not tested)
4. **Pagination:** Does `/searchOrg` handle pagination for large result sets? (Not tested)
5. **Search Filters:** What does the `type` parameter do in `/tim-kiem-theo-ten`? (Caused 500 error)

---

## Next Steps

1. **Test pagination** on `/searchOrg` with large result sets
2. **Analyze HTML** from `/website-tin-nhiem` to find if there's embedded JS for trust data
3. **Check for hidden XHR endpoints** by monitoring network traffic in real browser
4. **Evaluate scraping** the website listing pages as fallback
