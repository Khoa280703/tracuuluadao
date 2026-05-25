---
title: CloakBrowser Analysis & Cloudflare Turnstile Bypass Research
date: 2026-05-22
author: researcher
---

## Executive Summary

CloakBrowser is a **production-ready stealth Chromium fork** that solves your Cloudflare Turnstile headless challenge. It achieves 0.9 reCAPTCHA v3 score (human-level) vs 0.1 for stock Playwright through **48 C++ source-level patches**, not JavaScript injection. Drop-in Playwright/Puppeteer replacement with identical API. Free, no usage limits, tested April 2026 (Chromium 146).

---

## What is CloakBrowser?

A **modified Chromium binary** with 48+ C++ patches compiled in (canvas, WebGL, audio, fonts, GPU, WebRTC, automation signals). Distributed via npm/pip with auto-download (~200MB cached). Works identically local, Docker, VPS—no environment-specific patches.

**Key claim:** Passes 30/30 bot detection tests including Cloudflare Turnstile, FingerprintJS, BrowserScan, DataDome, PerimeterX.

---

## How It Bypasses Cloudflare Turnstile

**Core mechanism:** Source-level C++ patches survive Chrome updates (unlike JavaScript injection tools like playwright-stealth that break with every release).

**Why traditional approaches fail:**
- playwright-extra-stealth: 0.3-0.5 reCAPTCHA score, detectable patches
- undetected-chromedriver: 0.3-0.7 score, maintenance burden
- Stock Playwright: 0.1 score, **Turnstile blocks completely**

**CloakBrowser advantage:** Fingerprints modified at binary compilation—not injected, not configured—survives Chrome v145→v146 updates.

**April 2026 test results:** 0.9 reCAPTCHA v3 score, auto-resolves Turnstile that stock browsers fail entirely. FlareSolverr baseline: 90.38% success rate at 15.21s/bypass.

---

## API & Usage (Node.js)

### Installation
```bash
npm install cloakbrowser playwright-core
# OR for Puppeteer: npm install cloakbrowser puppeteer-core
```

### Basic Usage (Drop-in Replacement)
```javascript
import { launch } from 'cloakbrowser';

const browser = await launch();
const page = await browser.newPage();
await page.goto('https://checkscam.vn');  // Turnstile now passes
await browser.close();
```

### Configuration Options
- `proxy`: HTTP/SOCKS5 with auth (e.g., `'http://user:pass@proxy:8080'`)
- `headless`: Set `false` for headed mode (recommended for aggressive detection)
- `timezone` / `locale`: Auto-detected from proxy GeoIP if provided
- `args`: Extra Chrome flags as array

### CLI Utilities
```bash
npx cloakbrowser install    # Pre-download binary
npx cloakbrowser info       # Check status
npx cloakbrowser update     # Manual update check
```

---

## System Requirements & Headless Deployment

**Platforms:** Linux x86_64/arm64, macOS arm64/Intel, Windows x86_64.

**Linux headless (your use case):**
- Works truly headless (no xvfb required for basic Turnstile bypass)
- For aggressive detection (Kasada, Akamai), install fonts:
  ```bash
  apt install fonts-noto-color-emoji fonts-freefont-ttf fonts-unifont fonts-ipafont-gothic
  ```
- Docker image available: `cloakhq/cloakbrowser` (pre-configured, fonts included)
- For maximum stealth vs aggressive detection, use **headed mode with Xvfb** (free, virtual display):
  ```bash
  sudo apt install xvfb
  Xvfb :99 -screen 0 1920x1080x24 &
  export DISPLAY=:99
  ```

---

## Node.js Library vs Standalone Service

**CloakBrowser is a library + optional service:**
- **Library mode (primary):** npm module, works in-process, identical to Playwright API
- **Service mode (optional):** Can run as Docker container, accessed remotely
- **Your architecture:** Library mode fits perfectly—single Node.js process, no external service needed

---

## Comparison Table

| Tool | API | Approach | reCAPTCHA v3 | Turnstile | Maintenance |
|------|-----|----------|-------------|-----------|-------------|
| CloakBrowser | Playwright/Puppeteer | Source-level C++ | 0.9 | ✅ Auto | Low (binary updates only) |
| playwright-extra-stealth | Playwright extension | JS injection | 0.3-0.5 | ❌ Fails | High (breaks with Chrome updates) |
| undetected-chromedriver | Puppeteer-like | JS injection | 0.3-0.7 | ❌ Unreliable | High |
| FlareSolverr | HTTP proxy | Real browser backend | N/A | ✅ 90.38% | Low (external service) |
| Anti-detect browsers (paid) | Custom | Multi-profile | 0.9 | ✅ Yes | High cost ($49-299/mo) |

---

## For Your Checkscam.vn Use Case

**Recommendation:** CloakBrowser is the ideal fit.

**Why:**
1. Cloudflare Turnstile detected by checkscam.vn → CloakBrowser explicitly passes Turnstile
2. Headless Linux VPS → No xvfb required (works true headless), optional for extra stealth
3. Playwright codebase → Zero rewrite, swap import statement only
4. Free & open-source → No licensing, no quota limits
5. April 2026 tested → Latest Chromium 146, actively maintained

**Setup (5 min):**
```bash
npm install cloakbrowser  # replaces chromium in existing Playwright setup
```

Replace:
```javascript
import { chromium } from 'playwright'
const browser = await chromium.launch()
```

With:
```javascript
import { launch } from 'cloakbrowser'
const browser = await launch()
```

**Optional enhancements:**
- Add proxy for IP rotation if checkscam.vn blocks by IP
- Use `{ timezone: 'Asia/Ho_Chi_Minh' }` for locale fingerprint match

---

## Unresolved Questions

1. Does checkscam.vn use aggressive detection beyond Turnstile (Kasada, DataDome)? If yes, font installation + headed mode (Xvfb) may be needed.
2. Does checkscam.vn rate-limit or IP-block? CloakBrowser bypasses detection, not rate-limiting.
3. Performance requirements? CloakBrowser adds ~30-50ms startup overhead vs stock Playwright (negligible for most use cases).

---

## Sources

- [CloakBrowser GitHub Repository](https://github.com/CloakHQ/CloakBrowser)
- [CloakBrowser Official Documentation](https://cloakbrowser.dev/)
- [CloakBrowser npm Package](https://www.npmjs.com/package/cloakbrowser)
- [CloakBrowser Stealth Analysis - byteiota](https://byteiota.com/cloakbrowser-stealth-chromium-passes-all-bot-detection/)
- [Docker Image - cloakhq/cloakbrowser](https://hub.docker.com/r/cloakhq/cloakbrowser)
- [Comparison Guide - ZenRows](https://www.zenrows.com/blog/top-stealth-browser-tools)
- [Scrapfly Turnstile Bypass Guide](https://scrapfly.io/blog/posts/how-to-bypass-cloudflare-turnstile)
- [TechTimes Coverage - May 2026](https://www.techtimes.com/articles/316664/20260515/cloakhqs-open-source-chromium-fork-defeats-cloudflare-datadome-perimeterx-without-configuration.htm)
