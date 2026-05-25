# Phase 5: SEO + Polish

## Priority: Medium | Effort: S | Status: complete

## Overview

SSR meta tags, Open Graph, responsive fixes, loading states, error handling polish.

## Implementation Steps

### 1. SEO Meta Tags (`+page.svelte`)

```svelte
<svelte:head>
  <title>{query ? `${query} - Tra Cứu Lừa Đảo` : 'Tra Cứu Lừa Đảo - Kiểm tra SĐT, STK ngân hàng'}</title>
  <meta name="description" content="Tra cứu số điện thoại, số tài khoản ngân hàng, URL để kiểm tra rủi ro lừa đảo. Tổng hợp từ nhiều nguồn uy tín." />
  <meta property="og:title" content="Tra Cứu Lừa Đảo" />
  <meta property="og:description" content="Kiểm tra rủi ro lừa đảo cho SĐT, STK, URL" />
  <meta property="og:type" content="website" />
  <meta name="robots" content="index, follow" />
</svelte:head>
```

### 2. Loading States

- Search button: spinner + "Đang tra cứu..." text
- Empty state: illustration or text hướng dẫn sử dụng
- Error state: red banner with retry button
- Backend offline: "Hệ thống đang bảo trì" message

### 3. Responsive Polish

- Search box: `max-w-2xl mx-auto` on desktop, full-width mobile
- Source cards: 2-column grid desktop, 1-column mobile
- Detective report: readable line width (`max-w-prose`)
- Phase tracker: compact on mobile (icons only, no labels)

### 4. Accessibility

- Input label (sr-only)
- Risk badge: ARIA role="status"
- Focus management: auto-focus search on load
- Color contrast: verify dark mode meets WCAG AA

### 5. Static Assets

- `favicon.svg`: simple shield icon
- `robots.txt`: allow all
- Social preview image for OG

### 6. Error Boundary

Graceful handling:
- SSE connection lost → show "Kết nối bị gián đoạn" + auto-retry once
- Invalid query → client-side validation before submit
- Empty results → "Không tìm thấy thông tin" with suggestion text

## Related Files

- Modify: `frontend/src/routes/+page.svelte` (meta, states)
- Modify: `frontend/src/app.css` (responsive)
- Create: `frontend/static/favicon.svg`
- Create: `frontend/static/robots.txt`

## Success Criteria

- [x] Page title dynamic with query
- [x] OG tags render in link preview
- [x] Mobile layout usable (test 375px width)
- [x] Loading/error/empty states all handled
- [x] Lighthouse SEO score ≥ 90
- [x] No accessibility violations in axe scan
