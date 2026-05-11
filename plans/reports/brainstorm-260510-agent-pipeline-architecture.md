# Brainstorm Report: Agent Pipeline Architecture

**Date:** 2026-05-10 | **Status:** Finalized

---

## Problem

Cần thiết kế hệ thống agent orchestration cho pipeline "thám tử điều tra" — sử dụng self-hosted LLM (Qwen3.6-27B + Qwen3.5-4B trên 3x RTX 3090) để phân tích, tóm tắt, và tổng hợp dữ liệu scam từ nhiều nguồn. Yêu cầu: tối ưu hiệu năng tối đa, streaming real-time, trình bày detective-style.

---

## Quyết định đã chốt

| Quyết định | Chọn | Lý do |
|------------|------|-------|
| Orchestration | Rust trực tiếp gọi LLM API | Zero overhead, không thêm network hop |
| Agent scope | System prompt only, no function calling | Giảm token overhead, 4B model function calling không ổn định |
| Config style | Directory per agent (`config/agents/`) | Tách config/code, hot-reload prompts, clean structure |
| Model assignment | 4B cho tasks nhỏ, 27B cho synthesis | Tối ưu latency vs quality |
| Streaming | Stream mọi agent call | User thấy từng bước điều tra |
| Narrative style | Hybrid: facts + narrative | Mở đầu narrative, list bằng chứng, kết luận risk level |
| LLM endpoint | Direct mode (no safety filter) | Tránh false positive trên content scam, nhanh hơn |

---

## Architecture

### Agent Pipeline Flow

```
User query "0926408013"
    │
    ├─── Phase 1: Parallel Scraping (Rust, no LLM)
    │    ├── checkscam.vn      → raw HTML/reports
    │    ├── chongluadao.vn    → raw data
    │    ├── trangtrang.com    → raw data
    │    ├── tinnhiemmang.vn   → raw data
    │    └── Google/DDG        → search result URLs + snippets
    │
    ├─── Phase 2: Summarizer Agent (4B, parallel per source)
    │    Input: raw scraped content per source
    │    Output: JSON {summary, key_facts[], phone_mentions, risk_signals[]}
    │    Stream: từng summary → SSE to frontend
    │    Concurrency: parallel across sources, sequential within multi-page sources
    │
    ├─── Phase 3: URL Assessor Agent (4B, single call)
    │    Input: Google search results (title + snippet + URL) + Phase 2 context
    │    Output: JSON {urls: [{url, reason, priority}], skip_reason[]}
    │    Stream: "Chọn X/Y URL để điều tra sâu..."
    │
    ├─── Phase 4: Extractor Agent (4B, parallel per URL)
    │    Fetch: HTTP first → Lightpanda fallback for JS-only pages
    │    Input: fetched page content
    │    Output: JSON {summary, entities[], risk_signals[], related_numbers[]}
    │    Stream: từng extraction → SSE to frontend
    │
    └─── Phase 5: Detective Agent (27B, final synthesis)
         Input: ALL summaries + extractions from Phase 2-4
         Output: Streaming Markdown narrative
         Style: Hybrid (narrative intro → evidence list → risk conclusion)
```

### Model Assignment

| Agent | Model | Port | Temperature | Max Tokens | Response |
|-------|-------|------|-------------|------------|----------|
| Summarizer | Qwen3.5-4B | 8102 | 0.3 | 300 | JSON |
| URL Assessor | Qwen3.5-4B | 8102 | 0.1 | 200 | JSON |
| Extractor | Qwen3.5-4B | 8102 | 0.3 | 400 | JSON |
| Detective | Qwen3.6-27B | 8002 | 0.7 | 1500 | Streaming text |

### Latency Estimate

```
Phase 1 (scraping):     ~2-3s  (parallel, longest source wins)
Phase 2 (summarizer):   ~3-5s  (parallel, 4-6 sources × ~1s each, limited by concurrency)
Phase 3 (URL assessor): ~1-2s  (single call)
Phase 4 (extractor):    ~4-8s  (parallel, 3-5 URLs × ~2s each)
Phase 5 (detective):    ~8-15s (27B model, 1500 tokens streaming)
─────────────────────────────────
Total:                  ~18-33s (streaming makes wait tolerable)
```

---

## Config Directory Structure

```
config/
└── agents/
    ├── summarizer/
    │   ├── prompt.md           # System prompt: tóm tắt nội dung, extract facts
    │   ├── config.toml         # model=4B, temp=0.3, max_tokens=300
    │   └── examples.json       # 3-5 few-shot examples cho consistent output
    │
    ├── url-assessor/
    │   ├── prompt.md           # System prompt: đánh giá relevance, chọn URLs
    │   ├── config.toml         # model=4B, temp=0.1, max_tokens=200
    │   └── examples.json
    │
    ├── extractor/
    │   ├── prompt.md           # System prompt: extract info từ web page
    │   ├── config.toml         # model=4B, temp=0.3, max_tokens=400
    │   └── examples.json
    │
    ├── detective/
    │   ├── prompt.md           # System prompt: tổng hợp, viết narrative, risk level
    │   ├── config.toml         # model=27B, temp=0.7, max_tokens=1500
    │   └── examples.json       # 1-2 full example narratives
    │
    └── shared/
        ├── persona.md          # Shared detective persona instructions
        ├── risk-levels.md      # Risk level definitions & criteria
        └── output-schemas.json # JSON schemas cho structured output
```

### Config TOML Format

```toml
# config/agents/summarizer/config.toml
[model]
endpoint = "http://localhost:8102/v1/chat/completions"
name = "qwen3.5-4b"
temperature = 0.3
max_tokens = 300
top_p = 0.9

[response]
format = "json"                    # force JSON mode
schema = "shared/output-schemas.json#summarizer"

[prompt]
system = "prompt.md"               # relative to agent dir
include_shared = ["persona.md"]    # prepend shared prompts
few_shot = "examples.json"         # append as assistant/user examples

[runtime]
timeout_ms = 10000
retry_count = 1
stream = true                      # stream even JSON (for progress)
```

### Rust Loading Pattern

```
AgentConfig loaded at startup → cached in Arc<HashMap<String, AgentConfig>>
Hot-reload: watch config/ dir → reload on change (notify crate)
Prompt template: include shared/ files → inject query-specific context → send to LLM
```

---

## SSE Stream Protocol

### Event Types

```
event: phase_start
data: {"phase": 1, "label": "Thu thập dữ liệu", "total_sources": 5}

event: source_status
data: {"source": "checkscam.vn", "status": "done", "found": 3}
data: {"source": "chongluadao.vn", "status": "done", "found": 0}

event: phase_start
data: {"phase": 2, "label": "Phân tích dữ liệu", "total": 4}

event: summary_stream
data: {"source": "checkscam_report_1", "chunk": "Bài viết cảnh báo số 0926...", "done": false}
data: {"source": "checkscam_report_1", "chunk": " lừa đảo qua Shopee...", "done": false}
data: {"source": "checkscam_report_1", "chunk": "", "done": true, "result": {"risk_signals": ["shopee_scam"]}}

event: url_assessment
data: {"selected": 4, "total": 10, "urls": [{"url": "...", "reason": "Bài viết nhắc đến SĐT"}]}

event: extraction_stream
data: {"url": "https://example.com/...", "chunk": "Trang web cho thấy...", "done": false}

event: detective_stream
data: {"chunk": "## Kết quả điều tra\n\nSau khi phân tích...", "done": false}

event: complete
data: {"risk_level": "high", "confidence": 0.85, "sources_analyzed": 8, "duration_ms": 28400}
```

---

## Detective Agent Narrative Template (Hybrid)

```markdown
## Kết quả điều tra số {phone}

Sau khi phân tích {N} nguồn thông tin, tôi phát hiện số điện thoại này
xuất hiện trong {M} cảnh báo lừa đảo và {K} bài viết liên quan.

### Bằng chứng thu thập

#### Từ checkscam.vn (3 cảnh báo)
1. **"{report_title}"** — {summary}. [Xem nguồn]({url})
2. **"{report_title}"** — {summary}. [Xem nguồn]({url})

#### Từ Google Search (2 kết quả liên quan)
1. **"{page_title}"** — {summary}. [Xem nguồn]({url})

#### Từ chongluadao.vn
Không tìm thấy thông tin liên quan.

### Đánh giá rủi ro

**Mức độ: CAO** 🔴

Lý do:
- Xuất hiện trong 3 cảnh báo lừa đảo trên checkscam.vn
- Liên quan đến hình thức lừa đảo qua Shopee
- Nhiều nạn nhân báo cáo mất tiền từ 500K-2M VNĐ

### Khuyến nghị
- Không chuyển tiền cho số này
- Báo cáo lên cơ quan chức năng nếu bị liên hệ
```

---

## Risk Level Definitions

| Level | Label | Criteria |
|-------|-------|----------|
| **critical** | RẤT CAO 🔴 | 5+ cảnh báo, nhiều nguồn xác nhận, pattern rõ ràng |
| **high** | CAO 🟠 | 2-4 cảnh báo, hoặc 1 cảnh báo + nhiều bài Google liên quan |
| **medium** | TRUNG BÌNH 🟡 | 1 cảnh báo, hoặc xuất hiện trong context đáng ngờ |
| **low** | THẤP 🟢 | Không có cảnh báo, nhưng có ít thông tin trên mạng |
| **unknown** | CHƯA XÁC ĐỊNH ⚪ | Không tìm thấy thông tin nào |

**Quan trọng:** Không bao giờ kết luận "lừa đảo" — chỉ dùng "mức độ rủi ro" để tránh rủi ro pháp lý.

---

## Implementation Considerations

### Performance
- **Parallel all the things**: Phase 2 + Phase 4 đều parallel across sources/URLs
- **4B model concurrency**: vLLM trên 1 GPU xử lý ~4 concurrent requests → đủ cho parallel phases
- **27B sequential**: Chỉ 1 call cuối cùng, chiếm vRAM riêng
- **Prompt caching**: vLLM prefix caching — system prompt giống nhau giữa các calls → cache hit

### Resilience
- Agent timeout: 10s per call, fallback trả raw data nếu LLM fail
- Nếu tất cả LLM calls fail → vẫn trả scraped data thô cho user
- Phase 3 (URL Assessor) fail → visit tất cả URLs (fallback conservative)

### Hot-reload
- `notify` crate watch `config/agents/` → reload prompts/config runtime
- Cho phép iterate prompt engineering không cần restart server

### Caching
- Cache theo query (phone/bank/url) trong PostgreSQL
- TTL: 24h cho scrape results, 1h cho LLM analysis
- Cache key: hash(query + agent_version) → invalidate khi prompt thay đổi

---

## Unresolved Questions

1. **vLLM concurrent request limit** — 4B model trên 1 GPU xử lý bao nhiêu concurrent requests tối ưu? Cần benchmark.
2. **Few-shot examples impact** — Thêm examples tăng quality nhưng tăng token count → cần test trade-off trên 4B.
3. **Lightpanda production readiness** — Cần fallback plan nếu Lightpanda không stable cho visiting URLs.
4. **Context window usage** — Phase 5 Detective nhận ALL summaries — nếu nhiều sources, total input có thể vượt 4K tokens → cần truncation strategy.
5. **Prompt versioning** — Khi thay đổi prompt, cached results cũ có thể inconsistent → cache invalidation strategy cần design kỹ.
