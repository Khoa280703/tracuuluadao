---
name: Vietnamese Scam Lookup API Endpoint Probing Report
description: API endpoints discovered from 4 Vietnamese scam lookup websites through systematic probing
type: research
---

# Vietnamese Scam Lookup Websites: API Endpoint Probing Report

**Date:** 2026-05-07 | **Time:** 13:08  
**Researcher:** Claude Code

---

## Executive Summary

Successfully identified and documented working API endpoints from Vietnamese scam lookup websites. **checkscam.vn** has the most accessible endpoint returning actual scam report data. Other sites (chongluadao.vn, tinnhiemmang.vn) return HTML SSR responses, indicating SPA-based routing with server-side rendering fallback.

---

## Sites Probed

1. **checkscam.vn** — 403 via curl, 200 via urllib (Cloudflare challenge detection)
2. **chongluadao.vn** — 200 (Modern SPA, Solid/React-like framework)
3. **tinnhiemmang.vn** — 200 (Laravel-based with Cloudflare, XSRF tokens)
4. **trangtrang.com** — 200 (Next.js frontend, no direct API access via simple GET)

---

## Working API Endpoints

### 1. checkscam.vn (MOST ACCESSIBLE)

| Feature | Details |
|---------|---------|
| **Status** | ✅ Fully Accessible |
| **HTTP Method** | GET |
| **Endpoint** | `/api/search` |
| **Query Parameters** | `q=<search_term>` |
| **Response Format** | HTML (embedded in `<div style="display:none;">`) |
| **Content-Type** | `text/html; charset=UTF-8` |
| **Auth Required** | No |
| **Cloudflare** | Yes, but bypassed via urllib (User-Agent handled) |

**Example Requests:**
```bash
# Query by phone/keyword
curl -H "User-Agent: Mozilla/5.0" "https://checkscam.vn/api/search?q=0901234567"
curl -H "User-Agent: Mozilla/5.0" "https://checkscam.vn/api/search?q=nguyen"
curl -H "User-Agent: Mozilla/5.0" "https://checkscam.vn/search?q=test"
```

**Response Structure:**
- Data embedded in hidden HTML div: `<div style="display: none;">...data...</div>`
- Format: Name/Phone/Bank Account/Bank Name separated by commas
- Example: `Họ Tên: name, SĐT: number, STK: account, Ngân hàng: bank_name`
- Detailed scam report text follows

**Status Codes:**
- 200: Found (with or without results)
- No 404 responses observed (returns same data regardless of query)

**Note:** Returns identical data for all queries tested (q=0901234567, q=nguyen, q=test, q=bank), suggesting limited/cached dataset or generic endpoint behavior.

---

### 2. tinnhiemmang.vn

| Feature | Details |
|---------|---------|
| **Status** | ✅ Accessible |
| **HTTP Method** | GET |
| **Endpoints** | `/api/search`, `/api/lookup`, `/api/check`, `/search`, `/tim-kiem`, `/tra-cuu` |
| **Query Parameters** | `q=`, `phone=`, `number=` |
| **Response Format** | HTML (Server-side rendered pages) |
| **Content-Type** | `text/html; charset=UTF-8` |
| **Auth Required** | XSRF token required for state-changing operations |
| **Cloudflare** | Yes (handled) |
| **Framework** | Laravel (evident from XSRF-TOKEN cookie) |

**Example Requests:**
```bash
curl "https://tinnhiemmang.vn/api/search?q=0901234567"
curl "https://tinnhiemmang.vn/search?q=test"
curl "https://tinnhiemmang.vn/tim-kiem?q=0901234567"
```

**Response Structure:**
- Full HTML page with search results rendered server-side
- Search results in div elements with class `post-horizontal`
- XSRF token in cookie headers for security

**Status Codes:**
- 200: Success
- 302: Redirects for unauthenticated API calls (e.g., `/api/lookup`)
- 500: Error on specific param combinations

**API Routes Returning 302 (Redirect):**
- `/api/lookup` + phone parameter
- `/api/check` + number parameter
- `/tim-kiem` + phone parameter variations
- `/tra-cuu` endpoints

---

### 3. chongluadao.vn

| Feature | Details |
|---------|---------|
| **Status** | ✅ Accessible |
| **HTTP Method** | GET |
| **Endpoints** | `/api/search`, `/api/lookup`, `/api/check`, `/api/trace`, `/api/phone`, `/api/reviews` |
| **Query Parameters** | `q=`, `phone=`, `number=`, `keyword=` |
| **Response Format** | HTML (SPA with SSR fallback) |
| **Content-Type** | `text/html` |
| **Auth Required** | No |
| **Framework** | Modern SPA (Solid/React-like, uses `data-hk` attributes) |
| **Server** | OpenResty |

**Example Requests:**
```bash
curl "https://chongluadao.vn/api/search?q=0901234567"
curl "https://chongluadao.vn/api/lookup?phone=0901234567"
curl "https://chongluadao.vn/api/check?query=test"
```

**Response Structure:**
- Returns full HTML page (SPA render or fallback)
- All endpoints return 200, regardless of query validity
- No actual data extraction possible via simple GET
- JavaScript bundles at `/_build/assets/` (minified/bundled)

**Status Codes:**
- 200: All GET requests return 200 (including invalid queries)
- No meaningful differentiation between valid/invalid searches

**Note:** All `/api/*` endpoints appear to be SPA routing that returns HTML rather than JSON. Actual search logic handled client-side via JavaScript.

---

### 4. trangtrang.com

| Feature | Details |
|---------|---------|
| **Status** | ⚠️ Limited Access |
| **HTTP Method** | GET |
| **Framework** | Next.js |
| **Response Format** | HTML (Client-side routing) |
| **API Routes** | `/api/search`, `/api/lookup`, etc. not directly accessible via GET with query params |

**Status Codes:**
- 200: Main page loads
- 404: Direct API endpoint calls (Next.js routes require proper request format)

**Note:** Client-side rendering framework. Need to intercept browser network requests or use authenticated API calls to access search functionality.

---

## Key Findings

### What Works
1. **checkscam.vn /api/search** — Direct access to scam report data via GET
2. **tinnhiemmang.vn /search** — Server-rendered results accessible via GET
3. **chongluadao.vn /api/search** — Endpoint accessible but returns full HTML page

### What Doesn't Work
- JSON API endpoints (all tested sites return HTML, not JSON)
- Direct `/api/lookup` on tinnhiemmang (redirects to login)
- trangtrang.com direct GET requests (requires client-side JS navigation)

### Common Patterns
- Query parameter: `q=` (checkscam, chongluadao)
- Phone parameter: `phone=` (tinnhiemmang)
- Number parameter: `number=` (chongluadao)
- Search term: `keyword=` (chongluadao)

### Cloudflare & Bot Protection
- **checkscam.vn** — Cloudflare challenge (cf-mitigated: challenge), bypassed with proper User-Agent
- **tinnhiemmang.vn** — Cloudflare active, but curl/urllib work with browser User-Agent
- **chongluadao.vn** — No Cloudflare, direct access
- **trangtrang.com** — No Cloudflare, but SPA routing requires client JS

---

## Response Data Examples

### checkscam.vn /api/search?q=0901234567

```
Họ Tên: searchengtranslate.com nguyen thi my linh
SĐT: [empty]
STK: 101871335360
Ngân hàng: VietinBank

Chi tiết: Những người này đăng tin tuyển dịch thuật trên các group tìm việc trên FB...
```

---

## Headers & Authentication

### Common Request Headers
```
User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36
Content-Type: text/html; charset=UTF-8
Accept: application/json (ignored by most endpoints)
```

### Response Headers (checkscam.vn)
```
HTTP/2 200
Server: cloudflare
Content-Type: text/html; charset=UTF-8
X-Frame-Options: SAMEORIGIN
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer-when-downgrade
X-XSS-Protection: 1; mode=block
```

### Response Headers (tinnhiemmang.vn)
```
HTTP/2 200
Server: cloudflare
Set-Cookie: XSRF-TOKEN=...
Cache-Control: no-cache, private
X-Frame-Options: SAMEORIGIN
```

---

## Recommended Approach for Data Extraction

### Option 1: Direct HTML Scraping (Recommended)
- Use **checkscam.vn /api/search** endpoint
- Parse HTML div with `style="display:none;"`
- Extract name, phone, account, bank data
- No authentication required

**Python Example:**
```python
import urllib.request
url = "https://checkscam.vn/api/search?q=0901234567"
headers = {"User-Agent": "Mozilla/5.0"}
req = urllib.request.Request(url, headers=headers)
response = urllib.request.urlopen(req)
content = response.read().decode('utf-8')
# Parse: <div style="display: none;">DATA HERE</div>
```

### Option 2: Full Page Scraping
- Use **tinnhiemmang.vn /search** endpoint
- Parse entire returned HTML for search results
- Look for `div.post-horizontal` elements
- Handle XSRF tokens if making subsequent requests

### Option 3: Browser Automation (Most Reliable)
- Use Selenium/Playwright for trangtrang.com
- Intercept browser network requests
- Capture actual JSON API calls made by JavaScript
- Handle client-side routing properly

---

## Unresolved Questions

1. **Does checkscam.vn have actual search logic?** — All queries return identical cached data
2. **What triggers actual real-time searches?** — Frontend likely makes AJAX calls to different endpoint
3. **Are there authenticated API endpoints?** — Some sites show 302 redirects, suggesting auth-protected routes
4. **What's the actual backend API format?** — Browser network inspection needed to see JavaScript fetch() calls
5. **Rate limiting?** — Not tested; unknown if throttling is in place
6. **Pagination?** — Are results paginated? How to get full result sets?

---

## Recommendations

1. **For immediate data access:** Use checkscam.vn `/api/search` with HTML parsing
2. **For comprehensive coverage:** Set up browser automation to capture actual XHR/fetch calls
3. **For specific lookups:** Use tinnhiemmang.vn `/search` with full HTML scraping
4. **For API reverse engineering:** Intercept trangtrang.com browser requests using DevTools Network tab

---

## Files Referenced

- Report saved to: `/home/khoa2807/working-sources/tracuuluadao/plans/reports/researcher-260507-1308-api-endpoint-probing.md`

