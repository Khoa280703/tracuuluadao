# Brainstorm: Narrative UX Redesign

## Problem

Current UI hiển thị kiểu dashboard kỹ thuật (cards, stages, badges) — máy móc, không engaging. User muốn trải nghiệm "một điều tra viên đang truy vết real-time" — thông tin đến đâu kể đến đó.

## Approach: Hybrid Narrative + Evidence

### What Changes

**Frontend (major)**:
- Bỏ card/stage layout → single scroll narrative stream
- Phases 1-4: Template-based Vietnamese narrative (client-side)
- Phase 5: 27B detective stream → narrative style (not formal report)
- Evidence: expandable inline cards (link + summary, click for details)

**Backend (minor)**:
- Pipeline architecture unchanged (3×4B + 1×27B)
- SSE events unchanged
- Only change: detective agent prompt (formal → narrative)

### Frontend Narrative Templates (Phases 1-4)

Each SSE event maps to a narrative line:

```
phase_start(1)     → "Đang tiến hành phân tích số 0562015037..."
source_status(CheckScam, done, 10) → "🔍 Đã tìm thấy 10 cảnh báo trên CheckScam.vn, tiếp tục truy vết..."
source_status(TinNhiemMang, done, 0) → "Không tìm thấy thông tin trên TinNhiemMang."
source_status(Google, error) → (skip silently)
summary_result(CheckScam, {...}) → [expandable evidence card inline]
  "⚠️ CheckScam cho thấy số này có 17 báo cáo, liên quan đến mô hình giả trung gian game..."
phase_start(3)     → "Đang chọn các nguồn đáng tin cậy để điều tra sâu hơn..."
url_assessment     → "Đã xác định 5 URL cần truy vết chi tiết."
extraction_result  → [expandable evidence card]
  "📄 Trang checkscam.vn/nguyen-dinh-thang-21 xác nhận: tài khoản VP Bank 0562015037..."
phase_start(5)     → "Đã thu thập đủ dữ liệu. Đang tổng hợp kết luận..."
detective_stream   → [27B narrative stream — detective kể chuyện]
complete           → [risk badge sticky bottom]
```

### Detective Prompt Change

**Current** (formal report):
```
Viết báo cáo phân tích rủi ro... Nhóm bằng chứng theo nguồn...
```

**New** (investigator narrative):
```
Bạn là một điều tra viên chống lừa đảo. Hãy viết kết luận điều tra
bằng giọng tự nhiên, như đang kể cho người dùng nghe.
- Mở đầu: "Sau khi truy vết qua N nguồn, đây là những gì tôi tìm được..."
- Kể theo logic điều tra, không theo nguồn
- Dùng ngôn ngữ đời thường, tránh giọng pháp lý
- Kết thúc bằng khuyến nghị rõ ràng
```

### UI Layout

```
┌─────────────────────────────────────────────┐
│  🔍 [Search box]                            │
├─────────────────────────────────────────────┤
│                                             │
│  Đang tiến hành phân tích số 0562015037...  │
│                                             │
│  🔍 Đã tìm thấy 10 cảnh báo trên           │
│  CheckScam.vn, tiếp tục truy vết...        │
│                                             │
│  ┌─ ⚠️ CheckScam ──────────────────────┐   │
│  │ 17 báo cáo — giả trung gian game,   │   │
│  │ yêu cầu chuyển tiền cọc             │   │
│  │ ▼ Xem chi tiết                       │   │
│  └──────────────────────────────────────┘   │
│                                             │
│  Không tìm thấy thông tin trên              │
│  TinNhiemMang và ChongLuaDao.              │
│                                             │
│  Đang truy vết sâu hơn trên internet...    │
│                                             │
│  📄 Trang checkscam.vn xác nhận: tài khoản │
│  VP Bank liên quan đến Nguyễn Đình Thắng   │
│  ┌─ Evidence ────────────────────────────┐  │
│  │ checkscam.vn/nguyen-dinh-thang-21     │  │
│  │ ▼ Xem chi tiết                        │  │
│  └───────────────────────────────────────┘  │
│                                             │
│  Đã thu thập đủ dữ liệu. Tổng hợp...      │
│                                             │
│  🕵️ [27B streaming narrative conclusion]   │
│  Sau khi truy vết qua 6 nguồn, đây là      │
│  những gì tôi tìm được về số 0562015037... │
│  ▋ (typing cursor)                          │
│                                             │
├─────────────────────────────────────────────┤
│  🔴 RỦI RO RẤT CAO • Tin cậy 90% • 57s   │
└─────────────────────────────────────────────┘
```

### Implementation Scope

| Component | Effort | Change |
|-----------|--------|--------|
| `+page.svelte` | L | Rewrite layout → single narrative scroll |
| `sse-client.ts` | S | No change (events same) |
| `narrative-line.svelte` | M | New component — render 1 narrative line |
| `evidence-card.svelte` | M | Refactor from source-card — expandable |
| Detective `prompt.md` | S | Rewrite prompt to narrative style |
| `detective-report.svelte` | S | Keep (still markdown streaming) |
| Backend pipeline | None | No changes |
| Phase tracker | Delete | Replace with narrative flow |
| Source cards | Delete | Replace with evidence cards |

### Risks

- **Template narrative repetitive**: Same pattern every query. Mitigate: use 3-4 variants per event type, randomize.
- **Evidence card timing**: Summary results arrive async, may interleave oddly. Mitigate: queue and render in source order.

### Success Criteria

- [ ] No visible stages/cards — single narrative scroll
- [ ] User feels like "someone is investigating for me"
- [ ] Evidence inline with expandable details
- [ ] 27B detective conclusion reads naturally (not like a report)
- [ ] Mobile responsive (narrative works well on narrow screens)
- [ ] Performance: no regression in total investigation time

## Decision

**Hybrid narrative + evidence** with **template phases 1-4** + **27B narrative conclusion**. Minimal backend changes (only detective prompt). Major frontend rewrite.

## Next Steps

Create implementation plan with phases for frontend rewrite + detective prompt change.
