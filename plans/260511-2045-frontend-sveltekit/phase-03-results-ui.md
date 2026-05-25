# Phase 3: Results UI

## Priority: Critical | Effort: M | Status: complete

## Overview

Visual components for investigation progress and results: phase tracker, source cards, URL assessment, extraction cards, risk badge.

## Implementation Steps

### 1. Phase Tracker (`lib/components/phase-tracker.svelte`)

Horizontal stepper showing 5 phases:
```
① Thu thập  →  ② Phân tích  →  ③ Đánh giá URL  →  ④ Trích xuất  →  ⑤ Tổng hợp
```

- Current phase: highlighted + pulse animation
- Completed: checkmark + green
- Pending: gray
- Mobile: horizontal scroll if needed
- Shows progress message from `progress` events below active step

### 2. Source Cards (`lib/components/source-card.svelte`)

Card per source appearing one-by-one as `source_status` + `summary_result` arrive:

```
┌─────────────────────────────────────────┐
│ 🔍 checkscam.vn          3 kết quả  ✅ │
│ Số 0926... xuất hiện trong 3 cảnh báo   │
│ lừa đảo qua Shopee, thiệt hại 500K-2M  │
│                                         │
│ Dấu hiệu: shopee_scam, multiple_victims│
│ ▼ Chi tiết                              │
└─────────────────────────────────────────┘
```

- Initial state: source name + "Đang tải..." spinner (from `source_status`)
- Populated: summary + risk signals + expandable details (from `summary_result`)
- found=0: "Không tìm thấy" gray card
- Risk signals as colored tags

### 3. URL Assessment Section (`lib/components/url-assessment.svelte`)

Compact section showing selected URLs:
```
🔗 Chọn 4/10 URL để điều tra sâu
  • example.com/bai-viet... — "Bài viết nhắc đến SĐT" (priority 1)
  • forum.com/thread...     — "Diễn đàn thảo luận" (priority 2)
```

### 4. Extraction Cards (`lib/components/extraction-card.svelte`)

Similar to source cards but for URL extractions:
- URL as header (truncated, clickable)
- Summary + entities + risk signals
- Appear as `extraction_result` events arrive

### 5. Risk Badge (`lib/components/risk-badge.svelte`)

Sticky badge at bottom of viewport (or inline after complete):

```
┌──────────────────────────────────┐
│  🟠 RỦI RO CAO  •  Tin cậy 85% │
│  8 nguồn  •  28.4 giây          │
└──────────────────────────────────┘
```

Colors: critical=red, high=orange, medium=yellow, low=green, unknown=gray
Shows after `complete` event. Fades in with animation.

### 6. Layout Assembly (`+page.svelte`)

```
[Search Box]
[Phase Tracker]              ← appears on search start
[Source Cards grid]           ← phase 1-2
[URL Assessment]              ← phase 3
[Extraction Cards]            ← phase 4
[Detective Report]            ← phase 5 (Phase 4 of plan)
[Risk Badge]                  ← after complete
```

All sections use `{#if}` blocks gated on state. Smooth transitions with Svelte `transition:slide`.

## Related Files

- Create: `frontend/src/lib/components/phase-tracker.svelte`
- Create: `frontend/src/lib/components/source-card.svelte`
- Create: `frontend/src/lib/components/url-assessment.svelte`
- Create: `frontend/src/lib/components/extraction-card.svelte`
- Create: `frontend/src/lib/components/risk-badge.svelte`
- Modify: `frontend/src/routes/+page.svelte`

## Success Criteria

- [x] Phase tracker animates through phases correctly
- [x] Source cards appear one-by-one as data arrives
- [x] URL assessment section renders selected URLs
- [x] Risk badge shows correct color for risk level
- [x] Components responsive on mobile (stack vertical)
- [x] Smooth transitions between states
