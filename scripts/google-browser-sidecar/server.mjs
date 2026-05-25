import http from 'node:http';
import { BrowserPool } from './browser-pool.mjs';
import { benchmarkProxies } from './benchmark-proxies.mjs';
import { loadProjectEnvIfPresent } from './browser-search.mjs';

loadProjectEnvIfPresent();

const port = Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_PORT || '4417', 10);
const jobTimeoutMs = Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_JOB_TIMEOUT_MS || '25000', 10);
const pool = new BrowserPool({
  browserCount: Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_BROWSER_COUNT || '3', 10),
  slotsPerBrowser: Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_SLOTS_PER_BROWSER || '2', 10),
  checkscamBrowserCount: Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_CHECKSCAM_BROWSER_COUNT || '1', 10),
  proxyRegistryEnabled: process.env.GOOGLE_BROWSER_SIDECAR_PROXY_REGISTRY_ENABLED === '1'
});

await pool.start();

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === 'GET' && req.url === '/health') {
      return writeJson(res, 200, { ok: true, ...pool.snapshot() });
    }

    if (req.method === 'GET' && req.url === '/proxies') {
      return writeJson(res, 200, {
        ok: true,
        proxy_registry: pool.proxyRegistry?.snapshot() || null
      });
    }

    if (req.method === 'POST' && req.url === '/search') {
      const body = await readJsonBody(req);
      const payload = await withTimeout(
        pool.execute({
          query: body.query,
          queryType: body.queryType || 'phone',
          proxy: body.proxy || null,
          limit: body.limit || 30
        }),
        jobTimeoutMs,
        'browser sidecar job timed out'
      );
      return writeJson(res, 200, payload);
    }

    if (req.method === 'POST' && req.url === '/benchmark-proxies') {
      const body = await readJsonBody(req);
      const result = await benchmarkProxies({
        query: body.query || '0562015037',
        queryType: body.queryType || 'phone',
        limit: body.limit || 30,
        perFileLimit: body.perFileLimit,
        maxProxies: body.maxProxies,
        concurrency: body.concurrency
      });
      return writeJson(res, 200, result);
    }

    if (req.method === 'POST' && req.url === '/checkscam') {
      const body = await readJsonBody(req);
      const result = await withTimeout(
        pool.execute({ type: 'checkscam_search', query: body.query }),
        jobTimeoutMs,
        'checkscam search timed out'
      );
      return writeJson(res, result.ok ? 200 : 502, result);
    }

    if (req.method === 'POST' && req.url === '/checkscam/detail') {
      const body = await readJsonBody(req);
      const result = await withTimeout(
        pool.execute({ type: 'checkscam_detail', url: body.url }),
        jobTimeoutMs,
        'checkscam detail timed out'
      );
      return writeJson(res, result.ok ? 200 : 502, result);
    }

    writeJson(res, 404, { ok: false, error: 'not found' });
  } catch (error) {
    writeJson(res, 500, {
      ok: false,
      error: error instanceof Error ? error.message : String(error)
    });
  }
});

server.listen(port, () => {
  process.stdout.write(
    JSON.stringify({ ok: true, mode: 'unified-browser-pool', port, ...pool.snapshot() }) + '\n'
  );
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, async () => {
    server.close();
    await pool.close();
    process.exit(0);
  });
}

function writeJson(res, statusCode, payload) {
  res.statusCode = statusCode;
  res.setHeader('content-type', 'application/json; charset=utf-8');
  res.end(JSON.stringify(payload));
}

async function readJsonBody(req) {
  const chunks = [];
  for await (const chunk of req) {
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

async function withTimeout(promise, timeoutMs, message) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${message} after ${timeoutMs}ms`)), timeoutMs);
      })
    ]);
  } finally {
    clearTimeout(timer);
  }
}
