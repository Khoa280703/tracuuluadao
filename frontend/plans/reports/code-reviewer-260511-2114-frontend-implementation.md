## Code Review Summary

### Scope
- Files: 14 source files (9 Svelte components, 4 TypeScript modules, 1 CSS)
- LOC: ~500
- Focus: Full frontend implementation (SvelteKit 2 + Svelte 5 + Tailwind CSS 4)
- Build: 6 errors, 3 warnings from svelte-check

### Overall Assessment
Solid initial scaffold with correct Svelte 5 runes usage ($state, $derived, $effect, $props). The SSE event streaming architecture is well-designed. However, several critical issues prevent compilation, and the markdown rendering has an unmitigated XSS vulnerability. The code follows clean separation of concerns and has good type definitions.

---

### Critical Issues

**1. XSS via `{@html}` on unsanitized markdown output**

- File: `src/lib/components/detective-report.svelte`, line 20
- File: `src/lib/markdown.ts`, line 9-10

The detective report renders markdown from backend as raw HTML without sanitization. An attacker-controlled backend response or compromised markdown source can inject arbitrary scripts, event handlers, or malicious DOM elements.

```svelte
<!-- detective-report.svelte:20 -->
{@html html}
```

The custom link renderer in `markdown.ts:9-10` also uses string interpolation for `href` without escaping, which allows `javascript:` URL injection:

```ts
renderer.link = ({ href, text }) =>
  `<a href="${href}" target="_blank" rel="noopener noreferrer" class="...">${text}</a>`;
```

A malicious `href` like `javascript:alert(1)` bypasses `rel="noopener noreferrer"` and executes in the current context via `target="_blank"`.

**Fix:** Add DOMPurify sanitization:
```ts
import DOMPurify from 'dompurify';
export { marked };

// In detective-report.svelte:
let html = $derived(text ? DOMPurify.sanitize(String(marked.parse(text))) : '');
```
And in `markdown.ts`, filter `javascript:` URLs in the link renderer.

**2. `transition:slide` not imported -- compilation errors**

- Files: `+page.svelte:153`, `extraction-card.svelte:7`, `risk-badge.svelte:29`, `source-card.svelte:20,66`, `url-assessment.svelte:7`

All 6 instances of `transition:slide` fail because `slide` is not imported from `svelte/transition`. svelte-check reports: `"Cannot find name 'slide'."`

**Fix:** Add import to each affected file:
```ts
import { slide } from 'svelte/transition';
```

---

### High Priority

**3. SSE `error` event name conflicts with browser native EventSource `onerror`**

- File: `src/lib/sse-client.ts`, lines 57-59 and 61-64

The code registers both `es.addEventListener('error', ...)` and `es.onerror = () => { ... }`. The native EventSource dispatches `error` events on `onerror` for connection failures. If the backend sends an SSE event named `error`, both handlers may fire, or `onerror` may mask the backend error event.

**Fix:** Rename the backend error event to `investigation_error` to avoid collision, or remove `addEventListener('error', ...)` and rely solely on `onerror` for connection errors while using a differently named event for application errors.

**4. Silent swallowed parse errors in SSE client**

- File: `src/lib/sse-client.ts`, lines 31-59

All JSON parse failures are silently ignored with empty catch blocks. If the backend sends malformed JSON, events are silently dropped with no logging, making debugging nearly impossible.

**Fix:**
```ts
try { callbacks.onPhaseStart(JSON.parse(e.data)); }
catch (err) { console.warn('Failed to parse phase_start event:', err, e.data); }
```

**5. SSE connection leak on navigation away**

- File: `src/routes/+page.svelte`, lines 41, 120-123

When the user navigates away (e.g., browser back button), `sseHandle` is not cleaned up. The EventSource connection remains open, consuming server resources.

**Fix:** Add `$effect` teardown:
```ts
$effect(() => {
  return () => {
    sseHandle?.close();
  };
});
```

**6. Hardcoded backend URL in SSE proxy**

- File: `src/routes/api/investigate/+server.ts`, line 12

```ts
const backendUrl = `http://localhost:3000/api/investigate?${url.search}`;
```

Hardcoding `localhost:3000` breaks in production. No environment variable is used.

**Fix:** Use env variable:
```ts
const backendUrl = `${import.meta.env.VITE_BACKEND_URL || 'http://localhost:3000'}/api/investigate?${url.search}`;
```

**7. No request timeout on proxy fetch**

- File: `src/routes/api/investigate/+server.ts`, line 15

The `fetch(backendUrl)` call has no AbortController or timeout. If the backend hangs, the proxy request hangs indefinitely.

**Fix:**
```ts
const controller = new AbortController();
const timeout = setTimeout(() => controller.abort(), 300_000);
const response = await fetch(backendUrl, { signal: controller.signal });
clearTimeout(timeout);
```

---

### Medium Priority

**8. Svelte 5 `state_referenced_locally` warning -- non-reactive initial values**

- File: `src/routes/+page.svelte`, lines 25-26

```ts
let { data } = $props();
const initialQ = data.q as string;
const initialType = data.type as QueryType;
```

`data` is a reactive prop from `$props()`. The `const` assignments capture the initial value at declaration time. Svelte-check warns this is likely unintended. In this case it is intentional (initial URL params at load time), but the `as string` cast masks potential `undefined` from the PageLoad return type.

**Fix:** Add explicit null check and use a comment to suppress:
```ts
let { data } = $props();
const initialQ = (data?.q as string) || '';
const initialType = (data?.type as QueryType) || 'phone';
```

**9. URL type validation missing in `+page.ts`**

- File: `src/routes/+page.ts`, line 6

`type` from URL is used directly without validating it is one of `'phone' | 'bank' | 'url'`. A user can navigate to `?q=test&type=evil` and the frontend accepts `'evil'` as a QueryType, casting it at the component level.

**Fix:**
```ts
const validTypes = ['phone', 'bank', 'url'];
const typeVal = url.searchParams.get('type');
return {
  q: url.searchParams.get('q') ?? '',
  type: validTypes.includes(typeVal ?? '') ? typeVal : 'phone',
};
```

**10. `goto()` in `handleSearch` may cause infinite loop on URL change**

- File: `src/routes/+page.svelte`, lines 72-74 and 121-123

The `goto()` navigates to a new URL, which triggers a page load, which calls `handleSearch(initialQ, initialType)` again at line 121-123, which calls `goto()` again. This could cause an infinite redirect loop depending on SvelteKit's navigation handling.

**Fix:** Track whether the search was triggered from URL params:
```ts
let searchedFromUrl = $state(false);
// In handleSearch:
if (browser && !searchedFromUrl) {
  goto(...);
}
// In auto-start:
if (browser && initialQ && !searchedFromUrl) {
  searchedFromUrl = true;
  handleSearch(initialQ, initialType);
}
```

**11. `@tailwindcss/typography` prose styles conflict with custom link renderer**

- File: `src/app.css`, lines 29-31 and `src/lib/markdown.ts`, lines 9-10

The CSS defines `.prose a { @apply text-blue-600 ... underline; }` while the markdown renderer also inlines link styles. The CSS class will override the inline styles, making the renderer's style pointless. Worse, the CSS adds `underline` which the renderer doesn't apply, creating inconsistency between renderer output and prose styles.

**Fix:** Remove the inline style from the link renderer since the prose class handles it, or remove the CSS prose link override.

**12. No `key` in `{#each}` blocks for dynamic lists**

- Files: `source-card.svelte` (implied), `+page.svelte:195-198`, `+page.svelte:211-213`

The `{#each}` blocks for `sourceDisplay` and `extractions` lack keyed blocks. As items are appended during streaming, Svelte may reuse DOM nodes incorrectly.

**Fix:** Add key expressions:
```svelte
{#each sourceDisplay as sd (sd.source)}
{#each extractions as ext (ext.url)}
```

---

### Low Priority

**13. HTML lang is English, content is Vietnamese**

- File: `src/app.html`, line 2

`<html lang="en">` should be `<html lang="vi">` since all UI text is Vietnamese.

**14. `meta name="text-scale"` is not a standard meta tag**

- File: `src/app.html`, line 6

Likely a typo for `color-scheme` or a Tailwind CSS 4 specific setting. If it's for Tailwind's text-scaling mode, it should be `meta name="color-scheme" content="light dark"`.

**15. Missing `@types/node` or unused type reference**

- File: `tsconfig.json` (per svelte-check warning)

`Cannot find type definition file for 'node'` -- either install `@types/node` or remove `node` from `types` in tsconfig.

**16. Emoji usage in H1 and UI text**

- File: `+page.svelte:141`, multiple component files

Emojis in source code render differently across OS/font stacks. For production, consider using SVG icons or icon libraries for consistency.

**17. Error retry hardcoded to 'phone' type**

- File: `src/routes/+page.svelte`, line 167

```ts
onclick={() => handleSearch(currentQuery, 'phone')}
```

Retry always uses `'phone'` type instead of the original query type.

**Fix:** Track the current query type and use it:
```ts
let currentType = $state(initialType);
// Update in handleSearch: currentType = type;
// Retry: onclick={() => handleSearch(currentQuery, currentType)}
```

**18. No rate limiting on search**

- File: `src/lib/components/search-box.svelte`

Rapid successive searches create multiple SSE connections. While the previous connection is closed at `+page.svelte:66-69`, there's no debounce to prevent accidental double-clicks.

---

### Positive Observations

1. **Correct Svelte 5 runes usage** -- `$state`, `$derived`, `$effect`, `$props()` all used appropriately. Reactive array spreading (`[...phases, event]`) correctly triggers reactivity.
2. **Well-structured SSE event callback pattern** -- `createInvestigation` cleanly abstracts EventSource complexity into typed callbacks.
3. **Good type definitions** -- `types.ts` provides clear interfaces mirroring the backend SSE contract.
4. **Proper `rel="noopener noreferrer"`** on external links in extraction-card and url-assessment.
5. **Dark mode properly initialized** from `localStorage` and system preference with fallback.
6. **Accessible dark mode toggle** with `aria-label` and keyboard-focusable button.
7. **Progressive UI states** -- idle, loading, streaming, complete, error states all handled.
8. **`$derived` used for computed state** -- `sourceDisplay`, `detectedType`, `durationSec` all derive correctly.

---

### Recommended Actions (Priority Order)

1. Add `import { slide } from 'svelte/transition'` to all 6 files using `transition:slide` (blocks compilation)
2. Add DOMPurify for sanitizing markdown HTML output (security)
3. Sanitize `href` in markdown link renderer against `javascript:` URLs (security)
4. Rename SSE `error` event to avoid EventSource `onerror` collision
5. Add error logging to SSE parse catch blocks
6. Add `$effect` teardown for SSE connection cleanup on navigation
7. Parameterize backend URL with environment variable
8. Add `currentType` state to fix retry button using wrong type
9. Validate `type` query parameter in `+page.ts`
10. Add keyed `{#each}` blocks for streaming lists

---

### Metrics

- Type Coverage: 95% (all major types defined, minor issues with undefined handling)
- Test Coverage: 0% (no test files found)
- Linting Issues: 6 errors (missing slide imports), 3 warnings (state reference, missing @types/node)
- Svelte 5 Runes: Correctly applied
- Accessibility: Good (aria-labels present, semantic HTML, keyboard navigation on buttons)

### Unresolved Questions

1. Is the Rust backend expected to sanitize markdown content server-side, or should the frontend handle it?
2. What is the intended production backend URL? Is there an `.env.example` to reference?
3. Is the `text-scale` meta tag intentional for Tailwind CSS 4 font-scaling?
4. Should there be a connection timeout configured for the SSE stream?
