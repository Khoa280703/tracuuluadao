# Research Report: Google Search Scraping Strategy

**Date:** 2026-05-09 | **Status:** Finalized

---

## Problem

Google bắt buộc JavaScript từ 01/2025 — không có HTTP-only workaround nào hoạt động với request thông thường. Cần tìm phương án nhẹ nhất (không browser) để scrape Google Search results.

---

## Các phương án đã test

### Không hoạt động

| Phương án | Kết quả | Lý do |
|-----------|---------|-------|
| curl_cffi + chrome124 impersonate | ❌ JS wall | Google yêu cầu JS execution |
| `&gbv=1` (Google Basic Version) | ❌ JS wall | Không còn bypass JS |
| SOCS cookie + gbv=1 | ❌ JS wall | Cookie hết hạn/không hợp lệ |
| Googlebot User-Agent | ❌ JS wall | Google block luôn bot UA |
| AdsBot-Google User-Agent | ⚠️ No JS nhưng "Browser not supported" | Không trả kết quả |
| Lightpanda (headless browser nhẹ) | ❌ Thiếu Web APIs | `performance` API chưa implement, Google anti-bot JS crash |
| Full GSA headers + Sec-Fetch-* | ❌ 0% success | Headers conflict, Google detect giả mạo |
| Google Custom Search API | ❌ Deprecated | Đóng cho khách mới, ngưng 01/2027 |
| Google AJAX `/complete/search` | ⚠️ Chỉ autocomplete | Không trả search results |

### Hoạt động

| Phương án | Success Rate | Format | Browser cần? |
|-----------|-------------|--------|-------------|
| **GSA UA + `tch=1` + proxy** | **80% (0s delay)** | **JSON** | **Không** |
| GSA UA simple, no proxy, 2s delay | 75% | HTML | Không |
| GSA UA simple, no proxy, 0s delay | 45% | HTML | Không |
| DuckDuckGo HTML (curl_cffi) | ~95% | HTML | Không |
| chromiumoxide + Chrome headless | ~99% | HTML | Có (~400MB RAM) |

---

## Phương án chốt: GSA UA + tch=1 + Proxy + DuckDuckGo Fallback

### Cơ chế hoạt động

**SearXNG trick:** Google Search App (GSA) trên Android dùng User-Agent đặc biệt với suffix `NSTNWV`. Google nhận diện là app nội bộ → trả HTML/JSON thuần, không yêu cầu JavaScript.

**tch=1 parameter:** Kích hoạt Google internal RPC format — trả về chuỗi JSON objects concatenated thay vì HTML page. Mỗi object chứa key `d` là HTML fragment của 1 phần kết quả.

### Request config

```
URL: https://www.google.com/search?q={query}&hl=vi&gl=vn&tch=1
Headers:
  User-Agent: Mozilla/5.0 (Linux; Android 12; SM-S901U) AppleWebKit/537.36 
              (KHTML, like Gecko) Chrome/99.0.4844.88 Mobile Safari/537.36 NSTNWV
  Accept: */*
Proxy: rotating residential/datacenter proxy
```

### Response format (tch=1)

```
Content-Type: application/json
Body: Multiple JSON objects concatenated (not JSON array)

Each object: {"c": int, "d": "<html_fragment>", "e": str, "p": str, "u": str}
- "d" contains HTML with search results (<h3>, <a>, snippets)
- Parse all "d" values, concatenate, then extract h3/links via BeautifulSoup
```

### Parsing strategy

```python
def parse_google_tch1(response_text: str) -> list[dict]:
    decoder = json.JSONDecoder()
    pos = 0
    html_parts = []
    while pos < len(response_text):
        sub = response_text[pos:].lstrip()
        if not sub:
            break
        try:
            obj, end = decoder.raw_decode(sub)
            if isinstance(obj, dict) and "d" in obj:
                html_parts.append(obj["d"])
            pos += len(response_text) - len(sub) - pos + end
        except json.JSONDecodeError:
            pos += 1

    soup = BeautifulSoup("".join(html_parts), "html.parser")
    results = []
    for h3 in soup.select("h3"):
        parent_a = h3.find_parent("a")
        url = parent_a.get("href", "") if parent_a else ""
        # Google wraps URLs: /url?q=<actual_url>&sa=...
        if url.startswith("/url?q="):
            url = url.split("/url?q=")[1].split("&")[0]
        results.append({
            "title": h3.get_text(strip=True),
            "url": unquote(url),
        })
    return results
```

### Fallback chain

```
[1] Google Search (GSA UA + tch=1 + proxy)
    ├── Success → return results
    └── Fail (CAPTCHA/JS block/error)
        ↓
[2] Google Search retry (different proxy)
    ├── Success → return results
    └── Fail
        ↓
[3] DuckDuckGo HTML (curl_cffi, no proxy needed)
    ├── Success → return results
    └── Fail → return empty + log warning
```

---

## Benchmark kết quả

### GSA UA + tch=1 + proxy rotation (20 queries, 0s delay)

```
Success: 16/20 (80%)
CAPTCHA:  4/20 (20%)
JS Block: 0/20 (0%)
Format: JSON (faster parsing than HTML)
Avg response time: ~800ms
```

### Với fallback DuckDuckGo

```
Google success:     80%
+ DuckDuckGo retry: ~19% (95% of remaining 20%)
= Total coverage:   ~99%
```

### Production estimate (5K users/ngày)

```
Cache hit rate: ~70%
Actual queries: ~1,500/ngày = ~63/giờ = ~1/phút
Google success (80%): ~1,200 queries
DuckDuckGo fallback: ~300 queries
Total coverage: ~99%+
```

---

## GSA User-Agent pool

Rotate giữa nhiều UA để giảm detection. Source: SearXNG `gsa_useragents.txt`.

```
Mozilla/5.0 (Linux; Android 12; SM-S901U) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/99.0.4844.88 Mobile Safari/537.36 NSTNWV
Mozilla/5.0 (Linux; Android 11; KFTUWI) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.7680.165 Safari/537.36 NSTNWV
Mozilla/5.0 (Linux; Android 5.0; SM-G900P Build/LRX21T) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/39.0.1005.1041 Mobile Safari/537.36 NSTNWV
```

Key: suffix `NSTNWV` là bắt buộc — đây là marker Google Search App.

---

## DuckDuckGo fallback

```
URL: POST https://html.duckduckgo.com/html/
Body: q={query}+lừa+đảo
Headers:
  User-Agent: (any, curl_cffi impersonate chrome124)
  Content-Type: application/x-www-form-urlencoded
Proxy: không cần
```

### Parsing

```python
soup = BeautifulSoup(resp.text, "html.parser")
for r in soup.select(".result"):
    title = r.select_one(".result__a").get_text(strip=True)
    url = r.select_one(".result__url").get_text(strip=True)
    snippet = r.select_one(".result__snippet").get_text(strip=True)
```

### Đặc điểm

- Free, không cần proxy, không cần browser
- ~95% success rate
- Kết quả tiếng Việt tốt (đã test: "0926408013 lừa đảo" → 10 kết quả chính xác)
- Response time: ~800ms

---

## Tích hợp vào Rust backend

### Trong Rust (Axum), dùng `primp` crate (TLS impersonation cho Rust):

```
primp = "1.2"     # TLS impersonation (thay thế curl_cffi)
scraper = "0.22"  # HTML parsing (CSS selectors)
serde_json = "1"  # JSON parsing cho tch=1 response
```

### Flow trong backend

```
User request → Check cache (PostgreSQL)
  ├── Cache hit → return cached
  └── Cache miss → Fan-out parallel:
      ├── checkscam.vn     (primp, direct HTTP)
      ├── chongluadao.vn   (primp, direct HTTP)
      ├── trangtrang.com   (primp, direct HTTP)
      ├── tinnhiemmang.vn  (primp, direct HTTP)
      └── Google/DDG       (primp, GSA UA + tch=1 + proxy → DDG fallback)
```

---

## Proxy strategy

- **Google:** Dùng proxy rotation (Ola pool: 100 proxies) — mỗi request random proxy
- **4 scam sites:** Không cần proxy — curl_cffi/primp TLS impersonate bypass Cloudflare
- **DuckDuckGo:** Không cần proxy

### Proxy format (Ola)

```
ip:port:user:pass → http://user:pass@ip:port
```

---

## Rủi ro & Mitigation

| Rủi ro | Impact | Mitigation |
|--------|--------|-----------|
| Google block GSA UA trick | Mất Google search | DuckDuckGo fallback + monitor |
| CAPTCHA rate tăng | Giảm success rate | Thêm delay (2s), rotate proxy nhiều hơn |
| Google thay đổi tch=1 format | Parser break | Fallback sang HTML mode (không tch=1) |
| Proxy pool bị ban | Google queries fail | Rotate proxy thường xuyên, dùng residential |
| DuckDuckGo rate limit | Fallback fail | Cache aggressively, rate limit per IP |

---

## Unresolved Questions

1. `tch=1` response format có ổn định lâu dài? — cần monitor
2. Residential proxy có success rate cao hơn datacenter? — chưa test
3. `primp` Rust crate có hỗ trợ GSA UA trick tương đương curl_cffi? — cần verify
4. Google rate limit chính xác là bao nhiêu queries/phút/IP? — cần benchmark dài hơn
5. SearXNG có thêm trick nào khác ngoài GSA UA? — cần đọc sâu source code
