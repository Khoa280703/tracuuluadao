import { launch as cloakLaunch } from 'cloakbrowser';
import {
  loadProxiesFromDir,
  maskProxyUrl,
  parseProxyLine
} from './proxy-registry.mjs';
import {
  buildCloakLaunchOptions,
  loadProjectEnvIfPresent,
  readSidecarConfigFromEnv,
  runGoogleSearchJob
} from './browser-search.mjs';

loadProjectEnvIfPresent();

export async function benchmarkProxies(options = {}) {
  const config = readSidecarConfigFromEnv();
  const browser = await cloakLaunch(buildCloakLaunchOptions(config));

  try {
    const proxies = await selectProxies(options);
    const concurrency = Math.max(1, Number.parseInt(String(options.concurrency || 3), 10));
    const queue = proxies.slice();
    const results = [];

    await Promise.all(
      Array.from({ length: Math.min(concurrency, queue.length || 1) }, async () => {
        while (queue.length > 0) {
          const entry = queue.shift();
          const result = await testSingleProxy(browser, config, entry, options);
          results.push(result);
        }
      })
    );

    results.sort((left, right) => {
      if (left.success !== right.success) return left.success ? -1 : 1;
      return left.duration_ms - right.duration_ms;
    });

    return {
      query: options.query || '0562015037',
      query_type: options.queryType || 'phone',
      total_tested: results.length,
      successful: results.filter((item) => item.success).length,
      by_source_file: summarizeBySourceFile(results),
      results
    };
  } finally {
    await browser.close();
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const args = parseCliArgs(process.argv.slice(2));
  const payload = await benchmarkProxies({
    query: args.query || '0562015037',
    queryType: args['query-type'] || 'phone',
    limit: Number.parseInt(args.limit || '30', 10),
    perFileLimit: Number.parseInt(args['per-file-limit'] || '3', 10),
    maxProxies: Number.parseInt(args['max-proxies'] || '0', 10),
    concurrency: Number.parseInt(args.concurrency || '3', 10)
  });
  process.stdout.write(JSON.stringify(payload, null, 2) + '\n');
}

async function selectProxies(options) {
  const proxyDir = process.env.PROXY_DIR || './proxies';
  const perFileLimit = Number.parseInt(String(options.perFileLimit || 3), 10);
  let proxies = await loadProxiesFromDir(proxyDir, perFileLimit);
  if (options.maxProxies && options.maxProxies > 0) {
    proxies = proxies.slice(0, options.maxProxies);
  }
  return proxies;
}

async function testSingleProxy(browser, config, entry, options) {
  const startedAt = Date.now();
  try {
    const payload = await runGoogleSearchJob(
      browser,
      {
        query: options.query || '0562015037',
        queryType: options.queryType || 'phone',
        limit: options.limit || 30,
        proxy: entry.url
      },
      config,
      {
        proxy_source_file: entry.sourceFile,
        proxy_masked_url: maskProxyUrl(entry.url)
      }
    );

    return {
      source_file: entry.sourceFile,
      proxy: maskProxyUrl(entry.url),
      success: payload.success,
      results: payload.search_results.length,
      duration_ms: payload.metadata?.total_duration_ms || Date.now() - startedAt,
      transport_status: payload.metadata?.transport_status || 'unknown',
      google_status: payload.metadata?.google_status || 'unknown',
      content_status: payload.metadata?.content_status || 'unknown',
      first_url: payload.search_results[0]?.url || null
    };
  } catch (error) {
    return {
      source_file: entry.sourceFile,
      proxy: maskProxyUrl(entry.url),
      success: false,
      results: 0,
      duration_ms: Date.now() - startedAt,
      error: error instanceof Error ? error.message : String(error)
    };
  }
}

function summarizeBySourceFile(results) {
  const summary = {};
  for (const item of results) {
    if (!summary[item.source_file]) {
      summary[item.source_file] = { tested: 0, successful: 0 };
    }
    summary[item.source_file].tested += 1;
    if (item.success) summary[item.source_file].successful += 1;
  }
  return summary;
}

function parseCliArgs(args) {
  const out = {};
  for (let index = 0; index < args.length; index += 1) {
    const current = args[index];
    if (!current.startsWith('--')) continue;
    const key = current.slice(2);
    const next = args[index + 1];
    out[key] = !next || next.startsWith('--') ? '1' : next;
    if (next && !next.startsWith('--')) index += 1;
  }
  return out;
}
