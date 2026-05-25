import { chromium } from 'playwright-core';
import { loadProjectEnvIfPresent, readSidecarConfigFromEnv } from './browser-search.mjs';
import { loadProxiesFromDir, maskProxyUrl } from './proxy-registry.mjs';

loadProjectEnvIfPresent();
const config = readSidecarConfigFromEnv();
const proxies = await loadProxiesFromDir('/home/khoa2807/working-sources/tracuuluadao/proxies', 0);
const selectedProxies = ['proxy_MKVN.md', 'proxy_proxiesthatwork.md', 'proxy_ola.md']
  .map((source) => proxies.find((p) => p.sourceFile === source))
  .filter(Boolean);

const fingerprints = [
  {
    label: 'win-chrome-136',
    userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36',
    viewport: { width: 1365, height: 2140 },
    locale: 'vi-VN',
    timezoneId: 'Asia/Ho_Chi_Minh',
    acceptLanguage: 'vi-VN,vi;q=0.95,en-US;q=0.8,en;q=0.7',
    deviceScaleFactor: 1,
  },
  {
    label: 'mac-chrome-136',
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36',
    viewport: { width: 1512, height: 2100 },
    locale: 'en-US',
    timezoneId: 'Asia/Singapore',
    acceptLanguage: 'en-US,en;q=0.9,vi;q=0.6',
    deviceScaleFactor: 2,
  }
];

const queries = [
  '0562015037',
  'checkscam',
  'iphone 15',
  'thời tiết hà nội'
];
const starts = [0, 10, 20];

function toPlaywrightProxy(proxyUrl) {
  const parsed = new URL(proxyUrl);
  return {
    server: `${parsed.protocol}//${parsed.hostname}:${parsed.port}`,
    username: decodeURIComponent(parsed.username || ''),
    password: decodeURIComponent(parsed.password || '')
  };
}

function detect(currentUrl, body, resultCount) {
  const lower = `${currentUrl}\n${body}`.toLowerCase();
  const hard = [
    ['captcha', 'detected unusual traffic'],
    ['captcha', 'g-recaptcha'],
    ['captcha', '/sorry/index'],
    ['captcha', 'id="captcha"'],
    ['captcha', 'name="captcha"'],
    ['captcha', 'captcha-form'],
    ['blocked', 'consent.google']
  ];
  for (const [kind, needle] of hard) if (lower.includes(needle)) return kind;
  if (resultCount > 0) return 'serp';
  if (lower.includes('/httpservice/retry/enablejs')) return 'enablejs';
  return 'unknown';
}

async function testOne(browser, query, start, proxy, fp) {
  const context = await browser.newContext({
    locale: fp.locale,
    userAgent: fp.userAgent,
    viewport: fp.viewport,
    timezoneId: fp.timezoneId,
    colorScheme: 'light',
    deviceScaleFactor: fp.deviceScaleFactor,
    extraHTTPHeaders: { 'Accept-Language': fp.acceptLanguage },
    proxy: toPlaywrightProxy(proxy.url),
  });
  await context.route('**/*', (route) => {
    const t = route.request().resourceType();
    if (['image', 'media'].includes(t)) return route.abort();
    return route.continue();
  });
  const page = await context.newPage();
  const searchUrl = new URL('https://www.google.com/search');
  searchUrl.searchParams.set('q', query);
  searchUrl.searchParams.set('hl', 'vi');
  searchUrl.searchParams.set('gl', 'vn');
  searchUrl.searchParams.set('num', '10');
  searchUrl.searchParams.set('start', String(start));

  let status = 'unknown';
  let count = 0;
  try {
    await page.goto(searchUrl.toString(), { waitUntil: config.waitUntil, timeout: config.gotoTimeoutMs });
    await page.waitForSelector('h3', { timeout: config.selectorTimeoutMs }).catch(() => {});
    await page.waitForTimeout(config.fallbackSettleMs);
    const body = await page.content();
    count = await page.locator('h3').count();
    status = detect(page.url(), body, count);
  } catch (error) {
    status = String(error.message || error);
  }

  await page.close().catch(() => {});
  await context.close().catch(() => {});
  return {
    query,
    start,
    source: proxy.sourceFile,
    proxy: maskProxyUrl(proxy.url),
    fingerprint: fp.label,
    status,
    h3Count: count
  };
}

const browser = await chromium.launch({ executablePath: config.chromePath, headless: true, args: config.launchArgs });
const results = [];
for (const query of queries) {
  for (const start of starts) {
    for (const proxy of selectedProxies) {
      for (const fp of fingerprints) {
        results.push(await testOne(browser, query, start, proxy, fp));
      }
    }
  }
}
await browser.close();
console.log(JSON.stringify(results, null, 2));
