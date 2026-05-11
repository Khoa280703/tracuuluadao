# Research Report: checkscam.vn Scraping Approach

**Date:** 2026-05-07
**Purpose:** Document reverse-engineering findings for checkscam.vn — first target in scam lookup aggregation platform

---

## Context

Building a Vietnamese scam/fraud lookup aggregation website. checkscam.vn is the first source successfully reverse-engineered. This report covers all findings needed to implement a production scraper.

---

## Site Architecture

| Property | Value |
|----------|-------|
| Platform | WordPress (custom theme: `dkqh`) |
| Rendering | Server-rendered HTML (not SPA) |
| Dynamic features | jQuery AJAX |
| Protection | Cloudflare managed challenge |
| Bypass | `curl_cffi` with `impersonate="chrome124"` — 100% success rate |

---

## API Endpoints

### 1. Main Search — PRIMARY

```
GET https://checkscam.vn/?qh_ss=<keyword>
```

- Accepts: phone numbers, bank accounts, names, keywords
- Returns: Full HTML page with results embedded
- Auth: None required
- Cloudflare: Bypassed by `curl_cffi`
- Data included: report count, report links, summaries, trending scammers, latest comments
- Example: `?qh_ss=0926408013` → "Có 8 cảnh báo liên quan"

### 2. Report Detail Page

```
GET https://checkscam.vn/<slug>/
```

- Example: `/nguyen-bao-nam-18/`
- Returns: Full HTML with structured scam data
- Contains JSON-LD schema.org structured data
- Fields: Họ Tên, SĐT, STK, Ngân Hàng, nội dung tố cáo, lịch sử phản ánh, biệt danh

### 3. Rating/Stats Page

```
GET https://checkscam.vn/thongtin/<keyword>/
```

- Returns: Rating (x/5 stars), search count stats (today / yesterday / 7d / 30d), user reviews
- Example: `/thongtin/0926408013/` → "3/5 (5 đánh giá)"

### 4. WordPress AJAX API

```
POST https://checkscam.vn/wp-admin/admin-ajax.php
Content-Type: application/x-www-form-urlencoded
```

| Action | Params | Response | Auth |
|--------|--------|----------|------|
| `qh3_search` | `keyword`, `user_page`, `post_page`, `load_type` (both\|users\|posts), `topic_id` | JSON `{users_html, posts_html}` | No |
| `data_fetch` | `keyword` | Autocomplete suggestions | No |
| `qh_load_more_search` | `offset` | Paginated results | No |
| `kk-star-ratings` | — | Star rating submission | No |
| `qh_delete_search` | — | Delete search history | Yes |
| `qh_save_search2` | — | Save search | Yes |
| `load_more_follow` | — | More following tab content | Yes |
| `load_following_tab` | — | Following tab content | Yes |

---

## Data Structure (Report Detail Page)

| Field | Description |
|-------|-------------|
| Họ Tên | Full name |
| SĐT | Phone number |
| STK | Bank account number |
| Ngân Hàng | Bank name |
| Nội dung tố cáo | Report content / description |
| Biệt danh | Alias / nickname |
| Ngày đăng | Post date |
| Lượt xem | View count |
| Lịch sử phản ánh cùng STK | All reports sharing the same bank account |
| Facebook | Profile link (if available) |

---

## HTML Parsing Patterns

### Search Results Page (`/?qh_ss=`)

```python
# Report count
re.search(r'Có\s+(\d+)\s+cảnh báo', html)

# Report links (inside results section)
# Pattern: <a href="https://checkscam.vn/<slug>/">

# Each report item contains:
# - Name
# - Date: "DD tháng MM, YYYY"
# - View count: "XXX Lượt xem"
```

### Report Detail Page (`/<slug>/`)

```python
# Scam data in hidden div
# <div style="display: none;">Họ Tên: X, SĐT: X, STK: X, Ngân hàng: X</div>

# Visible labeled fields: "STK:", "Ngân Hàng:", etc.

# Structured data
soup.find('script', type='application/ld+json')

# Report content: main article body
```

---

## Recommended Scraping Approach

1. Use `curl_cffi` with `impersonate="chrome124"` — no Playwright needed
2. Search via `GET /?qh_ss=<keyword>`
3. Parse HTML to extract:
   - Report count: regex `Có\s+(\d+)\s+cảnh báo`
   - Report links: `<a href="https://checkscam.vn/<slug>/">`
   - Per-item: name, date, view count
4. For each report link, fetch detail page for full structured data
5. Optionally hit `/thongtin/<keyword>/` for rating/stats
6. Use AJAX `qh3_search` for supplementary user/post search when needed

---

## Performance Benchmarks

| Metric | Value |
|--------|-------|
| Cloudflare bypass rate | 100% (chrome124, chrome120, chrome110, safari17_0, edge101) |
| Single request latency | ~2.5s avg |
| 10 requests / 5 concurrent | 12.2s total (~1.2s avg per req) |
| RAM per curl_cffi session | ~5MB |
| RAM (Playwright equivalent) | ~300MB |
| Rate limiting detected | None at 5 concurrent workers |

---

## Scaling Considerations

- `curl_cffi` is lightweight — can scale to 20-50 concurrent workers without issue
- Add random delays (0.5–2s) between requests to reduce detection risk
- Rotate impersonation profiles (`chrome124`, `chrome120`, `safari17_0`, etc.) for stealth
- Reuse `curl_cffi` sessions for cookie persistence
- Cloudflare rate-limit threshold at higher concurrency: **untested** (see unresolved questions)

---

## Tested Phone Numbers

| Phone | Reports | Notes |
|-------|---------|-------|
| 0926408013 | 8 | Nguyen Bao Nam, VP Bank, lừa đảo |
| 0523854712 | 9 | Active scammer |
| 0367478849 | 3 | Active scammer |
| 088608553 | 3 | Active scammer |
| 9999999999196 | 4 | Active scammer |
| 0363293779 | 0 | Not in database |
| 0901234567 | 0 | Not in database |

---

## Unresolved Questions

1. Rate limiting threshold — untested above 5 concurrent workers
2. ~~Cookie/session expiry~~ — **Resolved**: curl_cffi bypass Cloudflare stateless, không cần cookie
3. Does checkscam.vn expose a sitemap with all report slugs?
4. IP-based blocking risk after sustained high-volume scraping
5. Additional search parameters — date range, category filters?
