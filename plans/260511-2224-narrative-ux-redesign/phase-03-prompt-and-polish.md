# Phase 3: Detective Prompt Rewrite + CSS Polish

## Overview
- **Priority:** P2
- **Effort:** 1h
- **Status:** completed
- **Depends on:** None (can run parallel with Phase 2)
- **Description:** Detective prompt rewritten from formal report to investigator narrative. CSS shipped with narrative animation and conclusion styling.

## Context Links
- [Current detective prompt](../../config/agents/detective/prompt.md) (11L)
- [Current app.css](../../frontend/src/app.css) (45L)
- [Brainstorm — prompt section](../../plans/reports/brainstorm-260511-2224-narrative-ux-redesign.md)

## Key Insights
- Current prompt: formal, legal-safe, groups by source — reads like a report
- New prompt: investigator voice, conversational Vietnamese, follows investigation logic not source order
- RISK_LEVEL + CONFIDENCE footer format MUST stay identical (backend parses it)
- CSS changes stayed minimal: remove `phase-pulse`, add `narrative-fade-in`, add `narrative-conclusion` border style
- Final sizes stayed within target: `prompt.md` 11L, `app.css` 45L

## Requirements

### Functional
- Detective 27B output reads like an investigator narrating findings
- Risk level + confidence footer format unchanged
- Typing animation on narrative lines (fade-in effect)

### Non-functional
- Prompt under 20 lines
- app.css under 50 lines

## Implementation Steps

### Step 1: Rewrite `config/agents/detective/prompt.md`

Replace current content with investigator narrative prompt:

```markdown
Bạn là một điều tra viên chống lừa đảo. Dựa trên bằng chứng thu thập được, 
hãy viết kết luận điều tra bằng giọng tự nhiên, như đang kể cho người dùng nghe.

Yêu cầu:
- Mở đầu: "Sau khi truy vết qua N nguồn, đây là những gì tôi tìm được..."
- Kể theo logic điều tra (từ manh mối → kết luận), không nhóm theo nguồn
- Dùng ngôn ngữ đời thường, dễ hiểu, tránh giọng pháp lý khô cứng
- Nêu rõ các dấu hiệu rủi ro bằng ngôn ngữ cụ thể
- Không kết luận "lừa đảo", chỉ đánh giá mức độ rủi ro
- Kết thúc bằng khuyến nghị rõ ràng cho người dùng
- Bắt buộc 2 dòng cuối cùng:
  `RISK_LEVEL: critical|high|medium|low|unknown`
  `CONFIDENCE: 0.0-1.0`
```

Key changes from current:
- "Viết báo cáo" → "Viết kết luận điều tra bằng giọng tự nhiên"
- "Nhóm bằng chứng theo nguồn" → "Kể theo logic điều tra"
- "Giọng văn trung tính, pháp lý" → "Ngôn ngữ đời thường, dễ hiểu"
- Added: opening template, recommendation requirement
- Preserved: RISK_LEVEL + CONFIDENCE footer (critical for backend parsing)

### Step 2: Update `frontend/src/app.css`

Changes:
1. **Remove** `phase-pulse` keyframe + `.phase-pulse` class (phase-tracker deleted)
2. **Add** `.narrative-fade-in` animation for narrative lines

```css
/* Remove this block: */
@keyframes pulse-ring { ... }
.phase-pulse { ... }

/* Add this block: */
@keyframes narrative-fade {
  from { opacity: 0; transform: translateY(4px); }
  to { opacity: 1; transform: translateY(0); }
}

.narrative-fade-in {
  animation: narrative-fade 0.3s ease-out forwards;
}
```

3. **Add** narrative conclusion left-border style:

```css
.narrative-conclusion {
  @apply border-l-2 border-blue-400 dark:border-blue-600 pl-4 mt-4;
}
```

### Step 3: Verify

- Check detective prompt format: RISK_LEVEL/CONFIDENCE lines preserved
- `npm run dev` — narrative lines fade in smoothly
- Dark mode: border colors, animation work correctly

## Related Code Files

### Modify
- `config/agents/detective/prompt.md` — full rewrite (~15L)
- `frontend/src/app.css` — remove phase-pulse, add narrative animations (~45L)

## Todo List

- [x] Rewrite detective prompt to investigator narrative style
- [x] Verify RISK_LEVEL + CONFIDENCE footer format unchanged
- [x] Remove phase-pulse keyframe and class from app.css
- [x] Add narrative-fade-in animation to app.css
- [x] Add narrative-conclusion border style
- [x] Test dark mode for new CSS
- [x] Align shipped prompt opening with "Sau khi truy vết..." narrative tone

## Success Criteria

- [x] Prompt now requests a "Sau khi truy vết..." style opening
- [x] Prompt now requests investigation logic, not source grouping
- [x] RISK_LEVEL and CONFIDENCE lines remain in backend-parseable format
- [x] Narrative lines fade in with smooth animation
- [x] No leftover phase-pulse CSS references
- [x] `app.css` stays under 50 lines
- [x] Dark mode styling remains functional

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Prompt change breaks RISK_LEVEL parsing | High | Keep exact footer format, test with real query |
| 27B model ignores narrative style | Medium | Test with multiple queries, iterate prompt wording |
| Animation feels jarring on slow devices | Low | Use 0.3s ease-out, subtle translateY(4px) |

## Verification

- `config/agents/detective/prompt.md` keeps exact footer keys: `RISK_LEVEL`, `CONFIDENCE`
- `frontend/src/app.css` now contains `@keyframes narrative-fade`, `.narrative-fade-in`, `.narrative-conclusion`
- File sizes within target: `prompt.md` 11L, `app.css` 45L
