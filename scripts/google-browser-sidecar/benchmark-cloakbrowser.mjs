import { launch as cloakLaunch } from 'cloakbrowser';

const TARGETS = [
  { name: 'Google', url: 'https://www.google.com/search?q=test&hl=vi' },
  { name: 'CheckScam', url: 'https://checkscam.vn/' },
  { name: 'ChongLuaDao', url: 'https://chongluadao.vn/' },
  { name: 'Lightweight', url: 'https://httpbin.org/html' },
];

const CONCURRENCY_LEVELS = [1, 2, 4, 6, 8, 10];
const BROWSER_COUNTS = [1, 2, 4, 6, 8, 10];

function formatMs(ms) { return `${Math.round(ms)}ms`; }
function formatMB(bytes) { return `${Math.round(bytes / 1024 / 1024)}MB`; }

async function getMemUsage() {
  const mem = process.memoryUsage();
  return { rss: mem.rss, heap: mem.heapUsed };
}

async function measureLaunchTime() {
  console.log('\n=== TEST 1: Browser Launch Time ===');
  const times = [];
  for (let i = 0; i < 3; i++) {
    const start = performance.now();
    const browser = await cloakLaunch({
      headless: true,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
      timezone: 'Asia/Ho_Chi_Minh',
      locale: 'vi-VN'
    });
    const elapsed = performance.now() - start;
    times.push(elapsed);
    console.log(`  Launch #${i + 1}: ${formatMs(elapsed)}`);
    await browser.close();
  }
  const avg = times.reduce((a, b) => a + b, 0) / times.length;
  console.log(`  Average launch: ${formatMs(avg)}`);
  return avg;
}

async function measurePageLoad(browser, target) {
  const page = await browser.newPage();
  const start = performance.now();
  try {
    await page.goto(target.url, { timeout: 20000, waitUntil: 'domcontentloaded' });
    const elapsed = performance.now() - start;
    const html = await page.content().catch(() => '');
    return { ok: true, elapsed, htmlLen: html.length, target: target.name };
  } catch (err) {
    return { ok: false, elapsed: performance.now() - start, error: err.message, target: target.name };
  } finally {
    await page.close().catch(() => {});
  }
}

async function measureSingleBrowserConcurrency(browser, concurrency, target) {
  const tasks = Array.from({ length: concurrency }, () => measurePageLoad(browser, target));
  const start = performance.now();
  const results = await Promise.all(tasks);
  const wallTime = performance.now() - start;

  const ok = results.filter(r => r.ok);
  const failed = results.filter(r => !r.ok);
  const avgTime = ok.length > 0 ? ok.reduce((a, r) => a + r.elapsed, 0) / ok.length : 0;
  const p95 = ok.length > 0 ? ok.map(r => r.elapsed).sort((a, b) => a - b)[Math.floor(ok.length * 0.95)] : 0;

  return { concurrency, wallTime, ok: ok.length, failed: failed.length, avgTime, p95 };
}

async function testConcurrentPages() {
  console.log('\n=== TEST 2: Concurrent Pages (Single Browser) ===');
  const browser = await cloakLaunch({
    headless: true,
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
    timezone: 'Asia/Ho_Chi_Minh',
    locale: 'vi-VN'
  });

  const target = TARGETS[3]; // httpbin - lightweight
  console.log(`  Target: ${target.name} (${target.url})`);

  for (const c of CONCURRENCY_LEVELS) {
    const result = await measureSingleBrowserConcurrency(browser, c, target);
    console.log(`  ${c} concurrent: wall=${formatMs(result.wallTime)} avg=${formatMs(result.avgTime)} p95=${formatMs(result.p95)} ok=${result.ok} fail=${result.failed}`);
  }

  await browser.close();
}

async function testMultipleBrowsers() {
  console.log('\n=== TEST 3: Multiple Browser Instances ===');
  const memBefore = await getMemUsage();

  for (const count of BROWSER_COUNTS) {
    const start = performance.now();
    const browsers = [];
    for (let i = 0; i < count; i++) {
      browsers.push(await cloakLaunch({
        headless: true,
        args: ['--no-sandbox', '--disable-dev-shm-usage', '--blink-settings=imagesEnabled=false'],
        timezone: 'Asia/Ho_Chi_Minh',
        locale: 'vi-VN'
      }));
    }
    const launchTime = performance.now() - start;
    const memAfter = await getMemUsage();

    // Each browser loads one page concurrently
    const target = TARGETS[3];
    const loadStart = performance.now();
    const results = await Promise.all(browsers.map(b => measurePageLoad(b, target)));
    const loadTime = performance.now() - loadStart;

    const ok = results.filter(r => r.ok).length;
    console.log(`  ${count} browsers: launch=${formatMs(launchTime)} load=${formatMs(loadTime)} ok=${ok}/${count} rss_delta=${formatMB(memAfter.rss - memBefore.rss)}`);

    for (const b of browsers) await b.close().catch(() => {});
    await new Promise(r => setTimeout(r, 1000));
  }
}

async function testRealWorldScenario() {
  console.log('\n=== TEST 4: Real-World Targets ===');
  const browser = await cloakLaunch({
    headless: true,
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--blink-settings=imagesEnabled=false'],
    timezone: 'Asia/Ho_Chi_Minh',
    locale: 'vi-VN'
  });

  for (const target of TARGETS) {
    const results = [];
    for (let i = 0; i < 3; i++) {
      results.push(await measurePageLoad(browser, target));
    }
    const ok = results.filter(r => r.ok);
    const avg = ok.length > 0 ? ok.reduce((a, r) => a + r.elapsed, 0) / ok.length : 0;
    const avgSize = ok.length > 0 ? ok.reduce((a, r) => a + r.htmlLen, 0) / ok.length : 0;
    const status = ok.length === 3 ? 'OK' : `${ok.length}/3`;
    console.log(`  ${target.name}: avg=${formatMs(avg)} size=${Math.round(avgSize / 1024)}KB status=${status}`);
    if (results.some(r => !r.ok)) {
      console.log(`    errors: ${results.filter(r => !r.ok).map(r => r.error).join(', ')}`);
    }
  }

  await browser.close();
}

async function testSustainedLoad() {
  const TOTAL_REQUESTS = 500;
  const CONFIGS = [
    { browsers: 2, slots: 2 },
    { browsers: 6, slots: 4 },
    { browsers: 10, slots: 4 },
  ];
  const target = TARGETS[3];

  console.log(`\n=== TEST 5: Sustained Load Comparison (${TOTAL_REQUESTS} requests each) ===`);
  console.log(`  Target: ${target.name} (${target.url})\n`);

  for (const cfg of CONFIGS) {
    const label = `${cfg.browsers}b x ${cfg.slots}s`;
    const browsers = [];
    for (let i = 0; i < cfg.browsers; i++) {
      browsers.push(await cloakLaunch({
        headless: true,
        args: ['--no-sandbox', '--disable-dev-shm-usage', '--blink-settings=imagesEnabled=false'],
        timezone: 'Asia/Ho_Chi_Minh',
        locale: 'vi-VN'
      }));
    }
    const launchRss = process.memoryUsage().rss;

    const allResults = [];
    let completed = 0;
    const workers = browsers.flatMap((b, bi) =>
      Array.from({ length: cfg.slots }, (_, si) => ({ browser: b, id: `b${bi}s${si}` }))
    );

    const start = performance.now();
    await Promise.all(workers.map(async (worker) => {
      while (completed < TOTAL_REQUESTS) {
        const idx = completed++;
        if (idx >= TOTAL_REQUESTS) break;
        const result = await measurePageLoad(worker.browser, target);
        allResults.push(result);
      }
    }));
    const totalTime = performance.now() - start;
    const peakRss = process.memoryUsage().rss;

    const ok = allResults.filter(r => r.ok);
    const sorted = ok.map(r => r.elapsed).sort((a, b) => a - b);
    const avg = ok.length ? ok.reduce((a, r) => a + r.elapsed, 0) / ok.length : 0;
    const p50 = sorted[Math.floor(sorted.length * 0.5)] || 0;
    const p95 = sorted[Math.floor(sorted.length * 0.95)] || 0;
    const p99 = sorted[Math.floor(sorted.length * 0.99)] || 0;
    const rps = (ok.length / totalTime) * 1000;

    console.log(`  [${label}] ${formatMs(totalTime)} total | ${rps.toFixed(2)} req/s | ${ok.length}/${TOTAL_REQUESTS} ok | avg=${formatMs(avg)} p50=${formatMs(p50)} p95=${formatMs(p95)} p99=${formatMs(p99)} | RSS=${formatMB(peakRss)}`);

    for (const b of browsers) await b.close().catch(() => {});
    await new Promise(r => setTimeout(r, 2000));
  }
}

async function testMemoryProfile() {
  console.log('\n=== TEST 6: Memory Profile ===');
  const memBase = process.memoryUsage();
  console.log(`  Base RSS: ${formatMB(memBase.rss)}`);

  const browser = await cloakLaunch({
    headless: true,
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
    timezone: 'Asia/Ho_Chi_Minh',
    locale: 'vi-VN'
  });
  console.log(`  +1 browser RSS: ${formatMB(process.memoryUsage().rss)}`);

  const pages = [];
  for (let i = 0; i < 5; i++) {
    const page = await browser.newPage();
    await page.goto('https://httpbin.org/html', { timeout: 15000, waitUntil: 'domcontentloaded' }).catch(() => {});
    pages.push(page);
    console.log(`  +${i + 1} pages RSS: ${formatMB(process.memoryUsage().rss)}`);
  }

  for (const p of pages) await p.close().catch(() => {});
  console.log(`  After closing pages RSS: ${formatMB(process.memoryUsage().rss)}`);

  await browser.close();
  console.log(`  After closing browser RSS: ${formatMB(process.memoryUsage().rss)}`);
}

// --- Run all tests ---
console.log('CloakBrowser Benchmark');
console.log(`System: ${(await import('os')).cpus().length} CPUs, ${Math.round((await import('os')).totalmem() / 1024 / 1024 / 1024)}GB RAM`);
console.log(`Node: ${process.version}`);

await measureLaunchTime();
await testConcurrentPages();
await testMultipleBrowsers();
await testRealWorldScenario();
await testSustainedLoad();
await testMemoryProfile();

console.log('\n=== DONE ===');
process.exit(0);
