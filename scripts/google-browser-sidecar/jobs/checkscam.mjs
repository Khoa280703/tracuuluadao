import { toPlaywrightProxy } from '../browser-search.mjs';

const CF_WAIT_TIMEOUT_MS = 20_000;
const CF_WAIT_PROXY_TIMEOUT_MS = 16_000;
const PAGE_LOAD_TIMEOUT_MS = 16_000;
const MAX_PROXY_ATTEMPTS = Number.parseInt(
  process.env.GOOGLE_BROWSER_SIDECAR_CHECKSCAM_PROXY_ATTEMPTS || '3',
  10
);

export async function runCheckscamSearchJob(browser, job, proxyRuntime) {
  const url = `https://checkscam.vn/?qh_ss=${encodeURIComponent(job.query)}`;
  return runWithProxyFallback(browser, url, proxyRuntime, 1000);
}

export async function runCheckscamDetailJob(browser, job, proxyRuntime) {
  return runWithProxyFallback(browser, job.url, proxyRuntime, 500);
}

async function runWithProxyFallback(browser, targetUrl, proxyRuntime, settleMs) {
  const directResult = await tryDirectPage(browser, targetUrl, settleMs);
  if (directResult.ok) return directResult;

  if (!proxyRuntime?.pickProxyEntry) return directResult;

  const seenIds = new Set();
  const seenSources = new Set();
  for (let i = 0; i < MAX_PROXY_ATTEMPTS; i += 1) {
    const proxyEntry = proxyRuntime.pickProxyEntry(seenIds, seenSources);
    if (!proxyEntry) break;
    seenIds.add(proxyEntry.id);
    if (proxyEntry.sourceFile) seenSources.add(proxyEntry.sourceFile);

    const result = await tryProxyPage(browser, targetUrl, proxyEntry.url, settleMs);
    if (result.ok) {
      proxyRuntime.markProxySuccess?.(proxyEntry, { target: 'checkscam' });
      return result;
    }
    proxyRuntime.markProxyFailure?.(proxyEntry, result.error);
  }
  return directResult;
}

async function tryDirectPage(browser, targetUrl, settleMs) {
  const page = await browser.newPage();
  try {
    await page.goto(targetUrl, { timeout: PAGE_LOAD_TIMEOUT_MS, waitUntil: 'domcontentloaded' });
    return await extractAfterCf(page, settleMs, CF_WAIT_TIMEOUT_MS);
  } catch (error) {
    return { ok: false, error: error.message, html: '' };
  } finally {
    await page.close().catch(() => {});
  }
}

async function tryProxyPage(browser, targetUrl, proxyUrl, settleMs) {
  const context = await browser.newContext({ proxy: toPlaywrightProxy(proxyUrl) });
  const page = await context.newPage();
  try {
    await page.goto(targetUrl, { timeout: PAGE_LOAD_TIMEOUT_MS, waitUntil: 'domcontentloaded' });
    return await extractAfterCf(page, settleMs, CF_WAIT_PROXY_TIMEOUT_MS);
  } catch (error) {
    return { ok: false, error: error.message, html: '' };
  } finally {
    await page.close().catch(() => {});
    await context.close().catch(() => {});
  }
}

async function extractAfterCf(page, settleMs, cfTimeout) {
  const cfPassed = await waitForCloudflare(page, cfTimeout);
  if (!cfPassed) {
    const html = await page.content().catch(() => '');
    if (html.length > 10000 && !isCloudflarePage(html)) {
      return { ok: true, html };
    }
    return { ok: false, error: 'cloudflare challenge not resolved', html: '' };
  }
  await new Promise((r) => setTimeout(r, settleMs));
  const html = await page.content().catch(() => '');
  return { ok: true, html };
}

async function waitForCloudflare(page, timeoutMs = CF_WAIT_TIMEOUT_MS) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const title = await page.title().catch(() => 'nav_error');
    const lower = title.toLowerCase();
    const currentUrl = page.url().toLowerCase();
    const html = await page.content().catch(() => '');
    const lowerHtml = html.toLowerCase();

    if (lower === 'nav_error') {
      await page.waitForLoadState('domcontentloaded').catch(() => {});
      await new Promise((r) => setTimeout(r, 2500));
      const retryHtml = await page.content().catch(() => '');
      if (retryHtml.length > 5000 && !isCloudflarePage(retryHtml)) {
        return true;
      }
      continue;
    }

    if (currentUrl.includes('/cdn-cgi/challenge-platform/')) {
      await new Promise((r) => setTimeout(r, 1800));
      continue;
    }

    if (
      !lower.includes('just a moment') &&
      !lower.includes('attention required') &&
      !lower.includes('chờ một chút') &&
      !lower.includes('please wait') &&
      !lower.includes('checking') &&
      !lower.includes('cloudflare') &&
      !isCloudflarePage(lowerHtml)
    ) {
      return true;
    }

    await new Promise((r) => setTimeout(r, 1200));
  }
  return false;
}

function isCloudflarePage(html) {
  const lower = html.toLowerCase();
  return (
    lower.includes('just a moment') ||
    lower.includes('chờ một chút') ||
    lower.includes('attention required') ||
    lower.includes('cf-challenge') ||
    (lower.includes('cloudflare') && lower.includes('ray id'))
  );
}
