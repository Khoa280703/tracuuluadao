# Brainstorm: AI Scam Lookup Platform

**Date:** 2026-05-07 | **Status:** Agreed

---

## Problem Statement

Người dùng VN cần kiểm tra SĐT/STK/URL/CCCD trước khi giao dịch online. Hiện tại phải tự search từng site (checkscam, chongluadao...) rồi tự đánh giá. Cần 1 nền tảng tổng hợp + AI phân tích tự động.

## Requirements

### Input types
- SĐT (0926408013)
- Số tài khoản ngân hàng (STK)
- URL/Website (http://sign-coin--base.pages.dev)
- CCCD (căn cước công dân)
- Bất kỳ identifier unique nào

### Output
- Data thô từ các nguồn (manh mối, bằng chứng)
- LLM suy luận + phân tích
- Mức độ rủi ro: "An toàn" / "Cần cẩn thận" / "Nguy cơ cao" / "Rủi ro rất cao"
- **KHÔNG kết luận "lừa đảo"** — chỉ trình bày evidence + risk level (tránh rủi ro pháp lý)

### Constraints
- Target: 5K users/ngày
- Self-hosted LLM: 3x RTX 3090 (2 NVLink)
- Realtime streaming response
- SEO tối đa (kết quả indexed trên Google)
- Monetization: tính sau

---

## Architecture

```
┌──────────────────────────────────────────────────┐
│  FRONTEND: Next.js (SSR/ISR for SEO)             │
│  - Search page (SĐT/STK/URL/CCCD input)         │
│  - Results page (SSR → Google indexed)           │
│  - Streaming UI (LLM analysis realtime)          │
│  - Risk score badge (An toàn → Rủi ro rất cao)   │
└─────────────────────┬────────────────────────────┘
                      │ HTTP/WebSocket
              ┌───────┴───────┐
              │ RUST BACKEND  │ Axum + Tokio
              │ (Orchestrator)│
              └───┬─────┬────┬┘
                  │     │    │
      ┌───────────┤     │    │
      │           │     │    │
┌─────┴────┐ ┌───┴──┐ ┌┴────────┐
│ Scrapers │ │  DB  │ │  LLM    │
│ (async)  │ │      │ │ Server  │
└────┬─────┘ └──────┘ └────┬────┘
     │                     │
┌────┴──────────────┐ ┌────┴────────┐
│ Sources:          │ │ vLLM/Ollama │
│ • checkscam.vn    │ │ Qwen3 12B   │
│ • chongluadao.vn  │ │ or Gemma4   │
│ • trangtrang.com  │ │ 3x RTX 3090 │
│ • tinnhiemmang.vn │ └─────────────┘
│ • Google (scrape) │
│ • Facebook        │
└───────────────────┘
```

### Flow chi tiết

```
User search "0926408013"
    │
    ▼
Rust Backend (Axum)
    │
    ├── [1] Check cache (Redis/PostgreSQL)
    │   └── Có cache < 24h? → Return cached result
    │
    ├── [2] Fan-out to ALL sources (parallel, tokio::spawn)
    │   ├── checkscam.vn   → GET /?qh_ss=0926408013 (curl, parse HTML)
    │   ├── chongluadao.vn → POST /checksafe/cld (JSON API)
    │   ├── trangtrang.com → GET /0926408013 (curl, parse HTML)
    │   ├── tinnhiemmang   → POST /searchOrg (limited)
    │   ├── Google Search  → Headless browser / scrape
    │   └── Facebook       → Headless browser / scrape
    │
    ├── [3] Stream raw results to frontend (SSE/WebSocket)
    │   └── Frontend renders evidence cards as they arrive
    │
    ├── [4] Once all sources collected → Build LLM prompt
    │   └── System prompt: "Phân tích evidence, đánh giá risk level"
    │
    ├── [5] Stream LLM response to frontend
    │   └── Risk level + reasoning + summary
    │
    └── [6] Cache complete result in DB
```

---

## Tech Stack

### Backend: Rust (Axum)
- **Why Rust:** Async scraping cực nhanh (tokio), memory safe, low footprint
- **HTTP client:** `reqwest` với TLS impersonation (tương tự curl_cffi)
- **HTML parsing:** `scraper` crate (CSS selectors)
- **Streaming:** SSE (Server-Sent Events) hoặc WebSocket
- **Queue:** Tokio channels cho internal task management

### Frontend: Next.js 15 (App Router)
- **Why Next.js:** SSR/ISR cho SEO, React ecosystem
- **Streaming:** React Suspense + Server Components
- **UI:** Tailwind CSS + shadcn/ui
- **SEO:** Pre-render popular scam numbers, sitemap, structured data

### Database: PostgreSQL
- Cache kết quả lookup (TTL 24h)
- User search history (anonymous)
- Aggregated statistics

### Cache: Redis
- Hot queries (SĐT phổ biến)
- Rate limiting per IP
- Session management

### LLM: vLLM + Qwen3 12B (hoặc Gemma4)
- **Why vLLM:** OpenAI-compatible API, continuous batching, high throughput
- **Model:** Qwen3 12B Q8 trên 1x 3090 (24GB vừa đủ)
  - Còn 2x 3090 NVLink cho headless browser + backup
- **Throughput estimate:** ~15-25 tokens/s per request, 3-5 concurrent
- **Alternative:** Gemma4 12B nếu Qwen3 không đủ tốt cho tiếng Việt

### Browser Automation: Playwright (Python/Node sidecar)
- Cho Google Search + Facebook scraping
- Chạy headless trên 1 trong 3 máy
- Pool 3-5 browser instances
- **Lưu ý:** Google rate limit ~100 searches/h, cần proxy rotation cho scale

---

## Data Sources Detail

### Đã reverse-engineer (curl_cffi, không cần browser)

| Source | Method | Data | Format |
|--------|--------|------|--------|
| checkscam.vn | `GET /?qh_ss=<q>` | SĐT/STK reports, tên, ngân hàng, nội dung tố cáo | HTML parse |
| chongluadao.vn | `POST /checksafe/<source>` | 14 nguồn check URL (cld, cyradar, apivoid...) | JSON |
| chongluadao.vn | `GET /checkphone?q=<sdt>` | Phone lookup (2 sources) | JSON |
| chongluadao.vn | `GET /checkwhois?q=<domain>` | WHOIS info | JSON |
| chongluadao.vn | `GET /checkbreach/email?q=<email>` | Email breach (HIBP) | JSON |
| trangtrang.com | `GET /<phone>` | Nhà mạng, cảnh báo cộng đồng | HTML parse |
| tinnhiemmang.vn | `POST /searchOrg` | Organization search (limited) | HTML |

### Cần browser automation

| Source | Method | Data |
|--------|--------|------|
| Google Search | Headless Playwright | Kết quả search "0926408013 lừa đảo" |
| Facebook | Headless Playwright | Public posts mentioning SĐT/STK |

---

## LLM Prompt Strategy

### System prompt
```
Bạn là chuyên gia phân tích rủi ro giao dịch trực tuyến tại Việt Nam.
Dựa trên các bằng chứng thu thập được, hãy:
1. Liệt kê các manh mối đáng chú ý
2. Phân tích mối liên hệ giữa các bằng chứng
3. Đánh giá mức độ rủi ro: An toàn / Cần cẩn thận / Nguy cơ cao / Rủi ro rất cao
4. Đưa ra lời khuyên cho người dùng

LƯU Ý: KHÔNG kết luận ai đó là "lừa đảo". Chỉ trình bày bằng chứng và mức độ rủi ro.
Nếu không có bằng chứng tiêu cực, kết luận "Chưa phát hiện dấu hiệu bất thường" 
nhưng vẫn khuyến cáo cẩn trọng.
```

### User prompt template
```
Phân tích thông tin sau về [SĐT/STK/URL]: {query}

=== NGUỒN: checkscam.vn ===
{checkscam_data}

=== NGUỒN: chongluadao.vn ===
{chongluadao_data}

=== NGUỒN: trangtrang.com ===
{trangtrang_data}

=== NGUỒN: Google Search ===
{google_results}

Hãy phân tích và đánh giá mức độ rủi ro.
```

---

## SEO Strategy

- **URL structure:** `/tra-cuu/0926408013` (SSR, indexable)
- **Meta tags:** "Kiểm tra SĐT 0926408013 - Có lừa đảo không?"
- **Sitemap:** Auto-generate cho các SĐT có reports
- **Schema.org:** FAQ + Review structured data
- **ISR:** Revalidate mỗi 24h cho cached results
- **Internal linking:** Related scam numbers, trending searches

---

## Scaling Considerations

### 5K users/ngày = ~210 requests/giờ = ~3.5 requests/phút

| Component | Capacity | Bottleneck? |
|-----------|----------|-------------|
| Rust backend | 10K+ req/s | ❌ Dư sức |
| Scraping (4 sites) | ~2-3s/request | ⚠️ Fan-out parallel OK |
| Google scrape | ~100/h (no proxy) | ⚠️ Cần proxy cho >100/h |
| LLM (Qwen3 12B) | ~5 concurrent | ⚠️ Queue needed cho peak |
| PostgreSQL cache | Hit rate ~60-70% | ✅ Giảm load đáng kể |
| Redis | 100K+ ops/s | ❌ Dư sức |

### Cache strategy
- Cache by query + 24h TTL
- Popular queries (top 1000 scam SĐT) pre-cached
- LLM response cached — same input = same output (deterministic temperature=0)

---

## MVP Phasing

### Phase 1: Core (2-3 tuần)
- Rust backend + 4 scraper sources
- Next.js frontend (search + results)
- LLM analysis (vLLM + Qwen3)
- PostgreSQL cache
- Streaming UI

### Phase 2: Enhancement (2 tuần)
- Google Search integration (Playwright sidecar)
- Facebook search
- SEO optimization (SSR, sitemap, schema.org)
- Rate limiting

### Phase 3: Scale (2 tuần)
- Redis cache layer
- Proxy rotation cho Google scrape
- User feedback (đánh giá kết quả)
- Admin dashboard (statistics)

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Source sites block IP | Mất data | Proxy rotation, respectful rate limiting |
| LLM hallucination | Sai risk level | "Risk level" language, disclaimer, temperature=0 |
| Google blocks scraping | Mất search data | Graceful degradation, DuckDuckGo fallback |
| 3090 hardware failure | LLM offline | Fallback to smaller model on remaining GPUs |
| Source sites change HTML | Parser break | Monitoring + alerts, modular parser design |

---

## Unresolved Questions

1. Qwen3 vs Gemma4 cho Vietnamese reasoning — cần benchmark
2. Google scrape proxy solution — self-hosted proxy hay service?
3. CCCD lookup — nguồn data nào? (chưa có site nào hỗ trợ)
4. Rate limit cho user — bao nhiêu queries/ngày/IP?
5. Rust TLS impersonation — crate nào tương đương curl_cffi?
