# Brainstorm: Layout Redistribute — Right Column Imbalance Fix

**Date:** 2026-05-24
**Status:** Agreed
**Scope:** `frontend/src/routes/+page.svelte`

## Problem Statement

Investigation page dùng 2-column grid (8:4 ratio):
- **Left (8 cols):** "Quá trình điều tra" — viewport-height capped (`lg:h-[calc(100vh-11.5rem)]`), internal scroll
- **Right (4 cols):** 4 cards stacked (Kết luận sơ bộ + Tóm tắt phát hiện + Đối tượng liên kết + Báo cáo cộng đồng) — no height constraint

**Symptoms:**
1. Cột phải dài hơn viewport → content bị khuất dưới fold
2. Scroll xuống xem cột phải → cột trái trống rỗng (đã hết ở viewport height)

**Root cause:** Chiều cao 2 cột mất cân bằng — trái cố định viewport, phải tự do dài vô hạn.

## Evaluated Approaches

| Approach | Pros | Cons | Verdict |
|----------|------|------|---------|
| A: Redistribute content | Giải quyết gốc rễ, KISS, không thay đổi scroll behavior | Bottom section cần thêm markup | **CHOSEN** |
| B: Sticky + scrollable right | Giữ nguyên 4 cards | Sticky + max-height gây vỡ flex layout bên trong (đã thử, lỗi "Tóm tắt" mất nội dung) | Rejected |
| C: Remove left fixed height | Cân bằng tự nhiên | Mất internal scroll experience, page quá dài | Rejected |
| D: Tabbed right column | Compact nhất | Thêm interaction overhead, user phải click để xem nội dung | Rejected |

## Final Solution: Redistribute Content

### Layout thay đổi

**Before:**
```
┌─────────────────────┬──────────┐
│  Left: Process       │ Kết luận │
│  (viewport height)   │ Tóm tắt  │
│                      │ Liên kết │  ← khuất
│                      │ Báo cáo  │  ← khuất
└─────────────────────┴──────────┘
```

**After:**
```
┌─────────────────────┬──────────┐
│  Left: Process       │ Kết luận │
│  (viewport height)   │ Tóm tắt  │
│                      │          │
└─────────────────────┴──────────┘
┌───────────────┬────────────────┐
│ Đối tượng      │ Báo cáo        │  ← chỉ hiện
│ liên kết       │ cộng đồng      │    khi hoàn tất
└───────────────┴────────────────┘
```

### Cụ thể

1. **Right column (lg:col-span-4):** Bỏ 2 cards cuối (Đối tượng liên kết + Báo cáo cộng đồng). Chỉ giữ:
   - Kết luận sơ bộ (risk meter)
   - Tóm tắt phát hiện (summary items)

2. **Bottom section (new, full-width):** Thêm sau `</div>` đóng grid 12-col:
   - Grid 2 cols: Đối tượng liên kết (trái) + Báo cáo cộng đồng (phải)
   - Responsive: stack trên mobile (grid-cols-1), 2 cols trên md+
   - **Condition:** `{#if completeEvent}` — chỉ hiện khi điều tra hoàn tất

### Ưu điểm
- Cột phải ngắn lại, cân bằng với cột trái
- Không thay đổi scroll behavior của cột trái
- Bottom section contextual — chỉ hiện khi cần
- KISS: chỉ move HTML blocks + wrap trong condition

### Risk
- **Nhỏ:** User quen vị trí cũ, cần visual cue hoặc scroll hint khi bottom section xuất hiện
- **Mitigation:** Có thể thêm subtle animation khi bottom section appears

## Implementation Estimate
- **Effort:** ~30 phút
- **Files changed:** 1 (`frontend/src/routes/+page.svelte`)
- **Risk:** Low — chỉ move HTML blocks, không thay đổi logic

## Success Criteria
- Cột phải không dài hơn cột trái trên desktop
- Kết luận sơ bộ + Tóm tắt phát hiện visible ngay without scroll
- Đối tượng liên kết + Báo cáo cộng đồng xuất hiện below fold khi điều tra hoàn tất
- Mobile: tất cả cards stack dọc bình thường
