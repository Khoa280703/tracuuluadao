# Phase 4: Detective Report + Dark Mode

## Priority: High | Effort: M | Status: complete

## Overview

Streaming Markdown detective narrative + dark mode toggle. The detective report is the centerpiece UX — text appears token-by-token like a detective writing in real-time.

## Implementation Steps

### 1. Detective Report (`lib/components/detective-report.svelte`)

Streams `detective_stream` events, renders Markdown incrementally:

```svelte
<script lang="ts">
  import { marked } from 'marked';

  let { text, done }: { text: string; done: boolean } = $props();

  let html = $derived(marked.parse(text));
</script>

<article class="prose dark:prose-invert max-w-none">
  {@html html}
  {#if !done}
    <span class="animate-pulse">▋</span>
  {/if}
</article>
```

Key behaviors:
- `marked.parse()` runs on every chunk update — re-renders full Markdown
- Typing cursor `▋` while streaming, disappears on `done`
- Auto-scroll to bottom as new content arrives
- `@tailwindcss/typography` `prose` class for Markdown styling

### 2. Markdown Styling

Configure `marked` for security + quality:

```typescript
// lib/markdown.ts
import { marked } from 'marked';

marked.setOptions({
  breaks: true,
  gfm: true,
});

// Sanitize: strip <script> tags from LLM output
const renderer = new marked.Renderer();
renderer.link = ({ href, text }) =>
  `<a href="${href}" target="_blank" rel="noopener noreferrer" class="text-blue-600 dark:text-blue-400">${text}</a>`;

marked.use({ renderer });

export { marked };
```

### 3. Dark Mode Toggle

Implementation using `class` strategy (not `media`):

```svelte
<!-- +layout.svelte -->
<script lang="ts">
  import { browser } from '$app/environment';

  let dark = $state(browser ? localStorage.getItem('theme') === 'dark'
    || (!localStorage.getItem('theme') && matchMedia('(prefers-color-scheme: dark)').matches)
    : false);

  $effect(() => {
    if (browser) {
      document.documentElement.classList.toggle('dark', dark);
      localStorage.setItem('theme', dark ? 'dark' : 'light');
    }
  });
</script>

<div class="min-h-screen bg-white dark:bg-gray-950 text-gray-900 dark:text-gray-100">
  <header class="flex justify-end p-4">
    <button onclick={() => dark = !dark}>
      {dark ? '☀️' : '🌙'}
    </button>
  </header>
  <slot />
</div>
```

### 4. Detective Report Sections Styling

Custom styles for detective output sections:

```css
/* Highlight risk level in detective report */
.prose strong:has(+ span) { /* Risk labels get colored */}
.prose h3 { @apply border-b border-gray-200 dark:border-gray-700 pb-2; }

/* Source links */
.prose a[href*="checkscam"],
.prose a[href*="chongluadao"] {
  @apply text-blue-600 dark:text-blue-400 underline;
}
```

### 5. Performance: Markdown Re-rendering

With streaming, `marked.parse()` runs on every chunk. Optimization:
- `marked.parse()` is fast (~1ms for 2KB text) — no debounce needed
- Only re-render when `text` actually changes (Svelte 5 handles this)
- If slow: buffer chunks and batch-render every 100ms

## Related Files

- Create: `frontend/src/lib/components/detective-report.svelte`
- Create: `frontend/src/lib/markdown.ts`
- Modify: `frontend/src/routes/+layout.svelte` (dark mode)
- Modify: `frontend/src/app.css` (prose styles)

## Success Criteria

- [x] Detective text streams token-by-token with typing cursor
- [x] Markdown headings, lists, links, bold render correctly
- [x] Source links open in new tab
- [x] Dark mode toggles correctly, persists in localStorage
- [x] Dark mode respects system preference on first visit
- [x] Auto-scroll follows streaming text
- [x] No XSS from LLM-generated Markdown
