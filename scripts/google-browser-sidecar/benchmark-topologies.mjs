import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { BrowserPool } from './browser-pool.mjs';
import { loadProjectEnvIfPresent } from './browser-search.mjs';

loadProjectEnvIfPresent();

const execFileAsync = promisify(execFile);
const topologies = (process.argv[2] || '1x10,2x5,3x3')
  .split(',')
  .map((item) => item.trim())
  .filter(Boolean);

const query = process.argv[3] || '0562015037';
const jobFactor = Number.parseFloat(process.env.GOOGLE_BROWSER_SIDECAR_BENCHMARK_FACTOR || '1');
const results = [];

for (const topology of topologies) {
  const [browserCount, slotsPerBrowser] = topology.split('x').map((item) => Number.parseInt(item, 10));
  const capacity = browserCount * slotsPerBrowser;
  const jobCount = Math.max(capacity, Math.round(capacity * jobFactor));
  const pool = new BrowserPool({
    browserCount,
    slotsPerBrowser,
    proxyRegistryEnabled: process.env.GOOGLE_BROWSER_SIDECAR_PROXY_REGISTRY_ENABLED === '1'
  });

  const startPoolAt = Date.now();
  await pool.start();
  const startupMs = Date.now() - startPoolAt;

  await pool.execute({ query, queryType: 'phone', limit: 30 }).catch(() => null);
  process.stderr.write(`benchmark ${topology}: capacity=${capacity} jobs=${jobCount}\n`);

  let peakRssKb = await sumDescendantRssKb(process.pid);
  const sampler = setInterval(async () => {
    peakRssKb = Math.max(peakRssKb, await sumDescendantRssKb(process.pid));
  }, 1000);

  const benchmarkStartedAt = Date.now();
  const jobs = Array.from({ length: jobCount }, (_, index) =>
    pool.execute({ query, queryType: 'phone', limit: 30 }).then(
      (payload) => ({
        success: payload.success,
        durationMs: payload.metadata?.total_duration_ms || 0,
        blocked:
          payload.metadata?.transport_status !== 'ok' ||
          ['captcha', 'enablejs', 'blocked'].includes(payload.metadata?.google_status)
            ? 1
            : 0
      }),
      (error) => ({
        success: false,
        durationMs: 0,
        blocked: 1,
        error: error instanceof Error ? error.message : String(error)
      })
    )
  );
  const jobResults = await Promise.all(jobs);
  const elapsedMs = Date.now() - benchmarkStartedAt;

  clearInterval(sampler);
  peakRssKb = Math.max(peakRssKb, await sumDescendantRssKb(process.pid));
  await pool.close();

  const durations = jobResults.map((item) => item.durationMs).filter(Boolean).sort((a, b) => a - b);
  const successCount = jobResults.filter((item) => item.success).length;
  results.push({
    topology,
    browser_count: browserCount,
    slots_per_browser: slotsPerBrowser,
    total_capacity: capacity,
    proxy_registry_enabled: Boolean(pool.proxyRegistry),
    startup_ms: startupMs,
    jobs_run: jobCount,
    jobs_succeeded: successCount,
    success_rate: jobCount === 0 ? 0 : successCount / jobCount,
    elapsed_ms: elapsedMs,
    jobs_per_second: elapsedMs === 0 ? 0 : Number((jobCount / (elapsedMs / 1000)).toFixed(2)),
    avg_job_ms: durations.length === 0 ? 0 : Math.round(durations.reduce((sum, value) => sum + value, 0) / durations.length),
    p95_job_ms: percentile(durations, 0.95),
    peak_rss_mb: Number((peakRssKb / 1024).toFixed(1)),
    blocked_queries_total: jobResults.reduce((sum, item) => sum + item.blocked, 0)
  });
}

process.stdout.write(JSON.stringify(results, null, 2) + '\n');

async function sumDescendantRssKb(rootPid) {
  const { stdout } = await execFileAsync('ps', ['-e', '-o', 'pid=,ppid=,rss=']);
  const rows = stdout
    .trim()
    .split('\n')
    .map((line) => line.trim().split(/\s+/).map((value) => Number.parseInt(value, 10)))
    .filter((parts) => parts.length === 3 && parts.every((value) => Number.isFinite(value)));

  const children = new Map();
  for (const [pid, ppid, rss] of rows) {
    if (!children.has(ppid)) children.set(ppid, []);
    children.get(ppid).push([pid, rss]);
  }

  let sum = 0;
  for (const [pid, , rss] of rows) {
    if (pid === rootPid) {
      sum += rss;
      break;
    }
  }
  const stack = [rootPid];
  const seen = new Set();
  while (stack.length > 0) {
    const pid = stack.pop();
    if (seen.has(pid)) continue;
    seen.add(pid);
    for (const [childPid, rss] of children.get(pid) || []) {
      sum += rss;
      stack.push(childPid);
    }
  }
  return sum;
}

function percentile(values, ratio) {
  if (values.length === 0) return 0;
  const index = Math.min(values.length - 1, Math.floor(values.length * ratio));
  return values[index];
}
