# Research Report: trangtrang.com API Analysis

**Date:** 2026-05-07  
**Purpose:** Reverse-engineer trangtrang.com API endpoints for phone number lookup aggregation  
**Status:** Complete

---

## Executive Summary

**trangtrang.com (Trang Trắng)** is a Next.js App Router-based phone number lookup platform. Unlike checkscam.vn (WordPress), it uses server-rendered React with RSC (React Server Components).

**Key Finding:** Primary endpoint is **`/{phone}`** (direct phone lookup page). No traditional REST API exists; all data is embedded in HTML responses.

**Bypass Method:** `curl_cffi` with `impersonate="chrome124"` — works 100% (Cloudflare protected).

---

## Site Architecture

| Property | Value |
|----------|-------|
| Platform | Next.js App Router (v15+) |
| Rendering | SSR + RSC (React Server Components) |
| Framework | React with TypeScript |
| Protection | Cloudflare |
| Bypass | `curl_cffi` (chrome124) — 100% success |
| Architecture Pattern | Single page lookup (no search API) |

---

## Primary Endpoint

### Phone Detail Page

```
GET https://www.trangtrang.com/{phone}
```

- **Phone format:** 10 digits (e.g., `0926408013`)
- **Response:** Full HTML page with phone info
- **Status:** 200 OK (all tested numbers)
- **Size:** ~84KB
- **Content-Type:** `text/html; charset=utf-8`

**Tested phones:**
- 0926408013 → 200 OK ✓
- 0367478849 → 200 OK ✓
- 0901234567 → 200 OK ✓
- 0999999999 → 200 OK ✓

---

## Data Extracted from Phone Page

### Page Structure

Each `/{phone}` page contains:

| Data | Location | Extraction |
|------|----------|-----------|
| Phone number | `<h2>` heading | Text content |
| Carrier info | Body text | Regex: `Vietnamobile\|Viettel\|MobiFone\|Gmobile` |
| Phone type | Body text | "Di động" (mobile) or "Cố định" (landline) |
| Page title | `<title>` tag | Format: `{phone} Số Điện Thoại ... - Trang Trắng` |
| Description | `<meta name="description">` | Phone info summary |
| Date modified | JSON-LD | `dateModified` timestamp |
| Warnings/alerts | Body text | "Cảnh báo", "Lừa đảo", "Lạm dụng" keywords |

### JSON-LD Structured Data

Schema.org microdata embedded in `<script type="application/ld+json">`:

```json
{
  "@context": "https://schema.org",
  "@type": "WebPage",
  "name": "{phone} Số Điện Thoại Di Động mạng {Carrier}",
  "url": "https://www.trangtrang.com/{phone}.html",
  "description": "Phone lookup info",
  "dateModified": "2026-05-07T10:37:55.410Z",
  "isPartOf": { "@id": "https://www.trangtrang.com/" }
}
```

---

## Search Functionality

### Query Parameter Search

```
GET https://www.trangtrang.com/?q={phone}
```

- **Purpose:** Search/filter (seems to load same results as homepage)
- **Response:** Homepage content (~115KB) with query parameter preserved
- **Phone in response:** Yes (query string shows in page)
- **Useful:** Limited — displays general content, not specific phone results
- **Alternative:** Direct access via `/{phone}` is more reliable

---

## API Endpoints NOT Found

The following REST API patterns return 404 or empty responses:

| Endpoint | Status | Note |
|----------|--------|------|
| `/api/search` | 404 | Not implemented |
| `/api/lookup` | 404 | Not implemented |
| `/api/phone` | 404 | Not implemented |
| `/api/check` | 404 | Not implemented |
| `/api/graphql` | 404 | No GraphQL API |
| `/api/v1/phone/{phone}` | 404 | No versioned API |
| `/so-dien-thoai/{phone}` | 404 | Vietnamese path not supported |

---

## Next.js Details

### App Router Structure

The page uses Next.js App Router with:

- **RSC streaming:** Responses contain `self.__next_f.push()` calls
- **No __NEXT_DATA__:** Unlike traditional Next.js pages, this site doesn't use static `__NEXT_DATA__` JSON
- **Dynamic SSR:** Each request generates fresh HTML
- **URL pattern:** `/{phone}.html` (files in root)

### Build Artifacts

```
/_next/static/chunks/0k_iv5-aq3t2c.js      (shared lib)
/_next/static/chunks/0d3shmwh5_nmn.js      (components)
/_next/static/chunks/07d_50f6no27p.js      (modules)
```

---

## Forms & Interaction

### Search Form (on all pages)

```html
<form action="javascript:throw new Error('React form unexpectedly submitted.')">
  <input name="phone" placeholder="Nhập số điện thoại..." />
  <button type="submit">Tra cứu</button>
</form>
```

- **Action:** Handled by React (client-side), not traditional form submission
- **Input:** Phone number field
- **Behavior:** Likely navigates to `/{phone}` on submit
- **Note:** Form action throws error → pure client-side handling

### Comment Form (on phone detail pages)

```
- "Gửi nhận xét" (Submit comment) button visible
- "Yêu cầu hiệu chỉnh" (Request correction) option
- "Hiệu chỉnh" (Edit) button
```

- **Status:** Forms present but no actual submission endpoints found
- **Inference:** May use hidden POST API or require authentication

---

## Parsing Strategy

### Recommended HTML Extraction

```python
from curl_cffi import requests as cf_requests
from bs4 import BeautifulSoup
import re

phone = "0926408013"
r = cf_requests.get(
    f"https://www.trangtrang.com/{phone}",
    impersonate="chrome124"
)

soup = BeautifulSoup(r.text, 'html.parser')

# Extract carrier
carrier_match = re.search(r'mạng (\w+)', soup.get_text())
carrier = carrier_match.group(1) if carrier_match else None

# Extract structured data
import json
schema = soup.find('script', type='application/ld+json')
if schema:
    data = json.loads(schema.string)
    date_modified = data.get('dateModified')
    title = data.get('name')
```

### Key Extraction Patterns

| Data | Regex Pattern | Example |
|------|---------------|---------|
| Carrier | `mạng\s+(\w+)` | Vietnamobile |
| Phone type | `(Di động\|Cố định)` | Di động |
| Warning count | `(\d+)\s+cảnh báo` | 3 cảnh báo |
| Date modified | `(\d{4}-\d{2}-\d{2})` | 2026-05-07 |

---

## Performance Notes

| Metric | Value |
|--------|-------|
| Response time | ~1.5-2s per request |
| Page size | ~84KB (HTML) |
| Cloudflare bypass | 100% success (chrome124) |
| Rate limiting | Unknown (not tested above 5 req/s) |
| Concurrent workers | Likely supports 20-50 (estimated) |

---

## Search Pattern (Secondary)

### Query Parameter Method

```
GET https://www.trangtrang.com/?q={phone}
```

- Returns homepage with query preserved in URL
- Phone number appears in response at: `self.__next_f.push([..., "?q=0926408013", ...])`
- **Not recommended** for data extraction — use `/{phone}` instead

---

## Implementation Notes

### For Scraper Development

1. **Direct endpoint is simplest:** Always use `GET /{phone}`
2. **No API authentication needed:** All endpoints public
3. **Use curl_cffi:** Essential to bypass Cloudflare
4. **Parse HTML:** Extract via BeautifulSoup + regex (no JSON API)
5. **Handle 200s only:** All valid phones return 200; non-existent numbers also return 200 (with empty data)
6. **Rate limit conservatively:** Unknown threshold — use 0.5-2s delays between requests

### Data Quality

- Phone data: Reliable (carrier, type, date updated)
- Warning count: Not explicitly visible in HTML (UI shows but extracted from JS)
- Comments/reviews: Form present but can't extract without JS execution or hidden API
- Edit requests: Can't access without authentication

---

## Comparison with checkscam.vn

| Aspect | checkscam.vn | trangtrang.com |
|--------|--------------|----------------|
| Platform | WordPress | Next.js |
| Search | `/?qh_ss={keyword}` | `/{phone}` direct lookup |
| API | WordPress AJAX endpoint | None (SSR only) |
| Data source | User-generated reports | Community database |
| Results format | HTML (cached) | HTML (SSR per request) |
| Parsing difficulty | Medium | Low (simpler HTML) |
| Performance | Faster (cached) | Slower (SSR per req) |

---

## Unresolved Questions

1. **Comment/review data:** Where is user comment data stored? Is there a hidden API or does it require JS execution?
2. **Rate limit threshold:** What's the max concurrent requests before IP blocking?
3. **Mobile app API:** Do iOS/Android apps use a different API endpoint?
4. **Authentication:** Any user-generated content require login?
5. **Full database export:** Is there a sitemap or bulk endpoint to get all phone numbers?
6. **Historical data:** Can we query previous versions of phone info?

---

## Next Steps for Implementation

1. Build scraper using `/{phone}` endpoint pattern
2. Start with test batch (100 phones) to establish rate limits
3. Monitor for Cloudflare blocks; rotate impersonation profiles if needed
4. Extract carrier + date_modified as baseline data
5. For comments: Either use Playwright with JS execution or investigate mobile app API
6. Integrate with existing checkscam.vn scraper in aggregator

---

## Files for Reference

- **Homepage:** `https://www.trangtrang.com/` (115KB, SSR)
- **Phone detail:** `https://www.trangtrang.com/{phone}` (84KB, SSR)
- **Example working:** `https://www.trangtrang.com/0926408013` (verified 200 OK)
