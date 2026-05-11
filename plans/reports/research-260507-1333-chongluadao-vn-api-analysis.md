# chongluadao.vn — API Analysis Report

**Date:** 2026-05-07 | **Status:** Complete

---

## Architecture

- **Frontend:** SolidJS SPA (SSR với SolidStart)
- **Backend API:** `https://feeds.chongluadao.vn` (REST JSON)
- **Cloudflare:** Không chặn curl_cffi — truy cập tự do
- **Auth:** Không cần (trừ checksafe cần reCAPTCHA)
- **Rate limit:** 429 handler trong code, chưa trigger trong test

## API Endpoints (JSON, không cần parse HTML)

### Không cần auth — Gọi trực tiếp

| Endpoint | Method | Params | Response | Hữu ích |
|----------|--------|--------|----------|---------|
| `/checkphone?q=<sdt>` | GET | q=SĐT | `[{source, data}]` (2 sources: scamvn, icallme) | ⭐ Tra cứu SĐT lừa đảo |
| `/checkwhois?q=<domain>` | GET | q=domain | `{owner, registrar, registration_date, expiration_date, nameservers}` | ⭐ WHOIS domain |
| `/checkbreach/email?q=<email>` | GET | q=email | `{email, breaches: [{Name, Domain, BreachDate, PwnCount}]}` | ⭐ Email breach (HIBP) |
| `/checkbreach/domain?q=<domain>` | GET | q=domain | Same as email | Domain breach |
| `/checkburneremail?q=<email>` | GET | q=email | `{is_burner_email: bool}` | Check email tạm |
| `/checkip?q=<ip>` | GET | q=IP | `{ip, version, blacklists: {engines}}` | IP blacklist check |
| `/checkfacebook?url=<fb_url>` | GET | url=FB URL | `{result}` | Check Facebook profile |
| `/net/check-url-exists?url=<url>` | GET | url=URL | `{exists: bool}` | URL tồn tại? |
| `/reports/check-exists?url=<url>` | GET | url=URL | `{exists: bool}` | URL đã bị report? |
| `/graphs/statictis` | GET | none | `{statuses: {CREATED, APPROVED, REJECTED}}` | Thống kê platform |
| `/posts?limit=N&sort=field,DIR` | GET | limit, sort | `{data: [{id, slug, translations, created_at}]}` | Tin tức/blog |
| `/ranks?limit=N&sort=field,DIR` | GET | limit, sort | `{data: [{email, total_points, rank}]}` | Top contributors |
| `/screenshots/capture?url=<url>` | GET | url=URL | base64 PNG image | Chụp ảnh website |
| `/net/ip?url=<url>` | GET | url=URL | `{ip, city, region, country, org}` | IP lookup (chậm) |

### Cần reCAPTCHA v3

| Endpoint | Method | Params | Mô tả |
|----------|--------|--------|-------|
| `/checksafe/<type>` | POST | `{url}` + header `X-Recaptcha-Token` | Kiểm tra URL an toàn |

- reCAPTCHA site key: `6LfPLGEsAAAAAEZ-a3o4Eeq5Jicay7_xPP_oIoAO`
- Đây là endpoint chính để check URL scam, nhưng bị lock sau recaptcha

## Gọi API mẫu

```python
from curl_cffi import requests as cf_requests

# Tra cứu SĐT
r = cf_requests.get("https://feeds.chongluadao.vn/checkphone?q=0926408013", impersonate="chrome124")
print(r.json())  # [{source: "scamvn", data: ...}, {source: "icallme", data: ...}]

# WHOIS
r = cf_requests.get("https://feeds.chongluadao.vn/checkwhois?q=shopee.vn", impersonate="chrome124")
print(r.json())  # {owner: "Shopee IP Singapore", registrar: "Web Commerce Communications LTD", ...}

# Email breach
r = cf_requests.get("https://feeds.chongluadao.vn/checkbreach/email?q=test@gmail.com", impersonate="chrome124")
print(r.json())  # {breaches: [{Name: "Adobe", PwnCount: 152445165, ...}]}
```

## So sánh với checkscam.vn

| Feature | checkscam.vn | chongluadao.vn |
|---------|-------------|----------------|
| **Response format** | HTML (cần parse) | JSON (sẵn sàng dùng) |
| **Tra cứu SĐT** | ✅ Có data phong phú | ⚠️ 2 sources nhưng trả null trong test |
| **Tra cứu STK** | ✅ Có | ❌ Không có endpoint |
| **Check URL scam** | ❌ Không | ✅ 14 sources, curl_cffi trực tiếp (server không validate reCAPTCHA) |
| **WHOIS** | ❌ Không | ✅ |
| **Email breach** | ❌ Không | ✅ (HaveIBeenPwned) |
| **IP blacklist** | ❌ Không | ✅ |
| **Screenshot** | ❌ Không | ✅ |
| **Cloudflare bypass** | Cần curl_cffi impersonate | Không cần (API direct) |
| **Ease of integration** | Trung bình (parse HTML) | Dễ (JSON API) |

## checksafe — URL Scam Check (14 nguồn)

**Endpoint:** `POST https://feeds.chongluadao.vn/checksafe/<source>`
**Body:** `{"url": "<url>"}`
**Header:** `X-Recaptcha-Token: <token>` (reCAPTCHA v3, site key: `6LfPLGEsAAAAAEZ-a3o4Eeq5Jicay7_xPP_oIoAO`)

### Sources (type param = tên nguồn, không phải loại input)

| Source | Mô tả | Kết quả test (sign-coin--base.pages.dev) |
|--------|-------|------------------------------------------|
| `cld` | ChongLuaDao internal DB | **malicious** ✅ |
| `cyradar` | CyRadar (VN cybersec) | **unsafe** ✅ |
| `apivoid` | APIVoid | **unsafe** ✅ |
| `scamadviser` | ScamAdviser | no_data |
| `criminalip` | CriminalIP | no_data |
| `phishtank` | PhishTank | no_data |
| `safebrowsing` | Google Safe Browsing | no_data |
| `ncsc` | NCSC Vietnam | no_data |
| `tinnhiemmang` | Tin Nhiệm Mạng | no_data |
| `scamvn` | ScamVN | no_data |
| `ipqualityscore` | IPQualityScore | no_data |
| `hudsonrock` | Hudson Rock | no_data |
| `bfore` | Bfore.ai | no_data |
| `phishdestroy` | PhishDestroy | no_data |

### Response format
```json
// cld source
{"status": 200, "message": "URL checked", "result": "malicious", "details": "URL in denylist"}
// other sources
{"data": {"status": 200, "message": "OK", "result": "unsafe|no_data|safe", "note": null}}
```

### reCAPTCHA — KHÔNG CẦN
- Frontend gửi reCAPTCHA token nhưng **server không validate**
- curl_cffi gọi trực tiếp không cần token, kết quả giống hệt
- **Không cần Playwright, không cần CAPTCHA service**

### Khi user search URL, frontend gọi song song:
1. 14x `POST /checksafe/<source>` (mỗi source 1 request)
2. `GET /net/ip?url=<url>` (IP lookup)
3. `GET /screenshots/capture?url=<url>` (screenshot)
4. `GET /checkbreach/domain?q=<domain>` (breach check)
5. `GET /checkwhois?q=<domain>` (WHOIS)
6. `GET /checkburneremail?q=<domain>` (burner check)

## Lưu ý

- `/checkphone` trả null cho tất cả SĐT test — có thể DB trống hoặc format SĐT khác
- checksafe cần reCAPTCHA nhưng Playwright giải tự động (v3 invisible)
- Rate limit 429 có trong code nhưng chưa trigger — nên giữ concurrency thấp
- API base `https://feeds.chongluadao.vn` — không phải domain chính

## Unresolved Questions

1. `/checkphone` DB có data không? Cần test với SĐT format `84xxxxxxxxx`?
2. Rate limit threshold là bao nhiêu?
3. ~~`/checksafe` types nào available?~~ — **Resolved**: 14 sources (cld, cyradar, apivoid, ...)
4. Có cách bypass reCAPTCHA v3 cho `/checksafe` không?
