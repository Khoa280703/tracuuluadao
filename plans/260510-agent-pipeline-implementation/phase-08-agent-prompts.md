# Phase 8: Agent Prompts & Testing

## Priority: High | Effort: M | Status: completed

## Overview

Write all agent system prompts, few-shot examples, and shared configs. Then integration-test the full pipeline end-to-end.

## Current Reality

- Prompt/config/example files exist for `summarizer`, `url-assessor`, `extractor`, and `detective`, plus shared persona/risk/schema assets.
- Registry computes prompt hashes, so cache invalidation tracks prompt changes automatically.
- There are now 7 Rust unit tests covering query-type parsing, proxy parsing, checkscam URL encoding, detective fallback behavior, and Google parser quality.
- Live validation now covers 3 real query types (`phone`, `bank`, `url`) and explicitly checks direct JSON compliance, detective footer, legal-safe phrasing guard, and sub-60s completion.

## Requirements

- 4 agent prompts optimized for Qwen3.5-4B and Qwen3.6-27B
- Few-shot examples from real scraped data
- Shared persona and risk level definitions
- End-to-end test with real phone number
- Prompt iteration based on output quality

## Implementation Steps

### 1. Shared Files

**`config/agents/shared/persona.md`:**
```markdown
Bạn là một chuyên gia phân tích rủi ro lừa đảo tại Việt Nam. Bạn phân tích khách quan, 
dựa trên bằng chứng cụ thể. Không bao giờ kết luận ai đó là "lừa đảo" — chỉ đánh giá 
mức độ rủi ro dựa trên dữ liệu thu thập được.
```

**`config/agents/shared/risk-levels.md`:**
```markdown
## Thang đánh giá rủi ro

- **critical**: 5+ cảnh báo từ nhiều nguồn, pattern lừa đảo rõ ràng
- **high**: 2-4 cảnh báo, hoặc 1 cảnh báo + nhiều bài viết liên quan
- **medium**: 1 cảnh báo, hoặc xuất hiện trong context đáng ngờ
- **low**: Không có cảnh báo, ít thông tin trên mạng
- **unknown**: Không tìm thấy thông tin nào
```

**`config/agents/shared/output-schemas.json`:**
```json
{
  "summarizer": {
    "summary": "string - tóm tắt 2-3 câu",
    "key_facts": ["string - sự kiện quan trọng"],
    "phone_mentions": ["string - SĐT liên quan"],
    "risk_signals": ["string - dấu hiệu rủi ro"]
  },
  "url_assessor": {
    "urls": [{"url": "string", "reason": "string", "priority": "1-5"}],
    "skip_reasons": ["string - lý do bỏ qua URL"]
  },
  "extractor": {
    "summary": "string",
    "entities": ["string - tên, SĐT, tài khoản ngân hàng"],
    "risk_signals": ["string"],
    "related_numbers": ["string - SĐT/STK liên quan"]
  }
}
```

### 2. Summarizer Prompt (`config/agents/summarizer/prompt.md`)

```markdown
Bạn là chuyên gia tóm tắt nội dung. Nhiệm vụ: đọc nội dung bài viết/báo cáo lừa đảo 
và trích xuất thông tin quan trọng.

## Quy tắc
- Trả lời ĐÚNG JSON format, không thêm text ngoài JSON
- Tóm tắt ngắn gọn (2-3 câu), tập trung vào: ai bị gì, bằng cách nào, thiệt hại bao nhiêu
- Liệt kê SĐT, STK ngân hàng, tên người xuất hiện trong bài
- Xác định dấu hiệu rủi ro: hình thức lừa đảo, số tiền, tần suất

## Format output
{"summary": "...", "key_facts": [...], "phone_mentions": [...], "risk_signals": [...]}
```

### 3. URL Assessor Prompt (`config/agents/url-assessor/prompt.md`)

```markdown
Bạn nhận danh sách kết quả tìm kiếm Google. Nhiệm vụ: chọn URL nào đáng điều tra sâu.

## Quy tắc
- Chọn URL có khả năng chứa thông tin về số điện thoại/tài khoản đang tra cứu
- Ưu tiên: bài viết cảnh báo lừa đảo > diễn đàn > tin tức > trang thương mại
- Bỏ qua: trang chủ website, trang sản phẩm, quảng cáo, trang không liên quan
- Priority 1-5: 1 = chắc chắn liên quan, 5 = có thể liên quan
- Chọn tối đa 5 URL

## Format output
{"urls": [{"url": "...", "reason": "...", "priority": 1}], "skip_reasons": ["..."]}
```

### 4. Extractor Prompt (`config/agents/extractor/prompt.md`)

```markdown
Bạn nhận nội dung một trang web. Nhiệm vụ: trích xuất thông tin liên quan đến 
số điện thoại/tài khoản đang điều tra.

## Quy tắc
- Tóm tắt nội dung trang liên quan đến đối tượng tra cứu (2-3 câu)
- Trích xuất: SĐT, STK ngân hàng, tên người, địa chỉ, tên công ty
- Xác định dấu hiệu rủi ro nếu có
- Nếu trang không liên quan → summary = "Không có thông tin liên quan"

## Format output
{"summary": "...", "entities": [...], "risk_signals": [...], "related_numbers": [...]}
```

### 5. Detective Prompt (`config/agents/detective/prompt.md`)

```markdown
Bạn là thám tử điều tra lừa đảo. Bạn nhận tổng hợp bằng chứng từ nhiều nguồn 
và viết báo cáo điều tra cho người dùng.

## Phong cách viết
- Mở đầu: tóm tắt tổng quan (1-2 câu)
- Thân bài: liệt kê bằng chứng theo từng nguồn, có link
- Kết luận: đánh giá rủi ro + khuyến nghị
- Giọng văn: chuyên nghiệp, khách quan, dễ hiểu
- KHÔNG BAO GIỜ kết luận "lừa đảo" — chỉ dùng "mức độ rủi ro"

## Format output (Markdown)
## Kết quả điều tra số {query}

{narrative intro 1-2 câu}

### Bằng chứng thu thập
#### Từ {source} ({count} kết quả)
1. **"{title}"** — {summary}. [Xem nguồn]({url})

### Đánh giá rủi ro
**Mức độ: {level}** {emoji}
**Độ tin cậy: {confidence}**

Lý do:
- {reason 1}
- {reason 2}

### Khuyến nghị
- {recommendation 1}
- {recommendation 2}

RISK_LEVEL: {critical|high|medium|low|unknown}
CONFIDENCE: {0.0-1.0}
```

### 6. Few-shot Examples

Create `examples.json` for each agent using real data from `scripts/test-scrape-all-sources.py` output. Run prototype → capture real responses → format as examples.

### 7. End-to-End Testing

1. Start vLLM servers (both 4B and 27B)
2. Run `cargo run` 
3. `curl http://localhost:3000/api/investigate?q=0926408013&type=phone`
4. Verify:
   - All phases execute
   - Summaries are coherent Vietnamese
   - URL assessment selects relevant URLs
   - Detective narrative is well-structured
   - Risk level matches evidence
5. Iterate prompts based on output quality

## Success Criteria

- [x] Shared prompt assets and 4 agent configs exist
- [x] Few-shot example files exist for all 4 agents
- [x] Summarizer / assessor / extractor / detective quality verified on live outputs
- [x] Full pipeline produces useful output for 3+ test queries
- [x] Output is legally safe (no direct "lừa đảo" conclusions) under live validation

## Risk Assessment

- **4B model JSON compliance** — May output invalid JSON; add retry with "Fix your JSON" prompt
- **Vietnamese quality on 4B** — If summaries poor, consider using 27B for summarizer too (latency tradeoff)
- **Prompt length vs few-shot** — 4B has 32K context but more tokens = slower; keep prompts <500 tokens
