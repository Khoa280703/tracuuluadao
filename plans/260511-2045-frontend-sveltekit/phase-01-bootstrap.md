# Phase 1: Project Bootstrap

## Priority: Critical | Effort: S | Status: complete

## Overview

Initialize SvelteKit project with Svelte 5, Tailwind CSS 4, TypeScript. Establish directory structure.

## Implementation Steps

1. Create SvelteKit project in `frontend/`:
   ```bash
   cd /home/khoa2807/working-sources/tracuuluadao
   npx sv create frontend --template minimal --types ts
   cd frontend
   npx sv add tailwindcss
   npm install marked
   ```

2. Configure Vite proxy to backend:
   ```typescript
   // vite.config.ts
   export default defineConfig({
     plugins: [sveltekit()],
     server: {
       proxy: {
         '/api': 'http://localhost:3000'
       }
     }
   });
   ```

3. Create directory structure:
   ```
   frontend/src/
   ├── routes/
   │   ├── +page.svelte           # Home: search + results
   │   ├── +page.ts               # Load URL params (q, type)
   │   └── +layout.svelte         # Root layout, dark mode toggle
   ├── lib/
   │   ├── components/            # UI components
   │   ├── sse-client.ts          # EventSource wrapper
   │   └── types.ts               # TypeScript types matching backend events
   ├── app.css                    # Tailwind + custom styles
   └── app.html
   ```

4. Set up Tailwind 4 dark mode in `app.css`:
   ```css
   @import "tailwindcss";

   @custom-variant dark (&:where(.dark, .dark *));
   ```

5. Create `lib/types.ts` matching backend SSE events:
   ```typescript
   export type QueryType = 'phone' | 'bank' | 'url';

   export interface AgentSummary {
     source: string;
     summary: string;
     key_facts: string[];
     phone_mentions: string[];
     risk_signals: string[];
   }

   export interface AgentExtraction {
     url: string;
     summary: string;
     entities: string[];
     risk_signals: string[];
     related_numbers: string[];
   }

   export interface SelectedUrl {
     url: string;
     reason: string;
     priority: number;
   }

   export type RiskLevel = 'critical' | 'high' | 'medium' | 'low' | 'unknown';

   // SSE event payloads
   export interface PhaseStartEvent { phase: number; label: string; total_sources?: number; }
   export interface SourceStatusEvent { source: string; status: string; found: number; }
   export interface ProgressEvent { phase: number; message: string; }
   export interface SummaryResultEvent { source: string; result: AgentSummary; }
   export interface UrlAssessmentEvent { selected: number; total: number; urls: SelectedUrl[]; }
   export interface ExtractionResultEvent { url: string; result: AgentExtraction; }
   export interface DetectiveStreamEvent { chunk: string; done: boolean; replace: boolean; }
   export interface CompleteEvent { risk_level: RiskLevel; confidence: number; sources_analyzed: number; duration_ms: number; }
   export interface ErrorEvent { phase?: number; message: string; recoverable: boolean; }
   ```

6. Verify: `npm run dev` starts on port 5173, proxy reaches backend

## Related Files

- Create: `frontend/` (entire project)
- Create: `frontend/src/lib/types.ts`

## Success Criteria

- [x] `npm run dev` starts without errors
- [x] Tailwind CSS working (test utility class)
- [x] Dark mode class toggle works
- [x] Vite proxy forwards `/api/*` to localhost:3000
- [x] TypeScript types match backend `InvestigationEvent` enum
