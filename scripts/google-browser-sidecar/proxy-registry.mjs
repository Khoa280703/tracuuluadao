import fs from 'node:fs/promises';
import path from 'node:path';

export class ProxyRegistry {
  constructor(options = {}) {
    this.proxyDir = options.proxyDir || process.env.PROXY_DIR || './proxies';
    this.cooldownMs = Number.parseInt(
      String(options.cooldownMs || process.env.GOOGLE_BROWSER_SIDECAR_PROXY_COOLDOWN_MS || 300000),
      10
    );
    this.maxPerFile = Number.parseInt(
      String(options.maxPerFile || process.env.GOOGLE_BROWSER_SIDECAR_PROXY_MAX_PER_FILE || 0),
      10
    );
    this.defaultSourceWeight = Number.parseInt(
      String(
        options.defaultSourceWeight || process.env.GOOGLE_BROWSER_SIDECAR_PROXY_DEFAULT_SOURCE_WEIGHT || 100
      ),
      10
    );
    this.sourceWeights = parseSourceWeights(
      options.sourceWeights || process.env.GOOGLE_BROWSER_SIDECAR_PROXY_SOURCE_WEIGHTS || '',
      this.defaultSourceWeight
    );
    this.disabledSourceFiles = parseSourceFileList(
      options.disabledSourceFiles ?? process.env.GOOGLE_BROWSER_SIDECAR_PROXY_DISABLED_SOURCES ?? ''
    );
    this.entries = [];
    this.started = false;
    this.cursor = 0;
  }

  async start() {
    if (this.started) return;
    this.entries = await loadProxiesFromDir(this.proxyDir, this.maxPerFile, this.disabledSourceFiles);
    this.started = true;
  }

  pick(options = {}) {
    if (this.entries.length === 0) return null;
    const now = Date.now();
    const excludedIds = normalizeSet(options.excludedIds);
    const excludedSourceFiles = normalizeSet(options.excludedSourceFiles);
    const available = this.entries.filter((entry) => entry.cooldownUntil <= now);
    const source = (available.length > 0 ? available : this.entries).filter(
      (entry) =>
        !excludedIds.has(entry.id) &&
        !excludedSourceFiles.has(entry.sourceFile)
    );
    const ranked = source
      .map((entry) => ({
        entry,
        score: scoreProxyEntry(entry, this.sourceWeights, this.defaultSourceWeight)
      }))
      .sort(
        (left, right) =>
          right.score - left.score ||
          left.entry.lastUsedAt - right.entry.lastUsedAt ||
          left.entry.id.localeCompare(right.entry.id)
      );

    if (ranked.length === 0) return null;
    const topScore = ranked[0].score;
    const topEntries = ranked.filter((item) => item.score === topScore);
    const index = this.cursor % topEntries.length;
    this.cursor += 1;
    return topEntries[index].entry;
  }

  markSuccess(entry, metadata = {}) {
    entry.successCount += 1;
    entry.lastSuccessAt = Date.now();
    entry.lastError = null;
    entry.lastDurationMs = metadata.durationMs || null;
    entry.lastUsedAt = Date.now();
    entry.cooldownUntil = 0;
  }

  markFailure(entry, error) {
    entry.failureCount += 1;
    entry.lastError = truncateError(error);
    entry.lastUsedAt = Date.now();
    entry.cooldownUntil = Date.now() + this.cooldownMs;
  }

  snapshot() {
    const now = Date.now();
    const summary = {};
    for (const entry of this.entries) {
      if (!summary[entry.sourceFile]) {
        summary[entry.sourceFile] = {
          total: 0,
          healthy: 0,
          successes: 0,
          failures: 0
        };
      }
      const bucket = summary[entry.sourceFile];
      bucket.total += 1;
      if (entry.cooldownUntil <= now) bucket.healthy += 1;
      bucket.successes += entry.successCount;
      bucket.failures += entry.failureCount;
    }
    return {
      proxy_dir: this.proxyDir,
      proxies_total: this.entries.length,
      proxies_ready: this.entries.filter((entry) => entry.cooldownUntil <= now).length,
      disabled_source_files: [...this.disabledSourceFiles],
      default_source_weight: this.defaultSourceWeight,
      configured_source_weights: Object.fromEntries(this.sourceWeights),
      by_source_file: summary
    };
  }
}

export async function loadProxiesFromDir(
  proxyDir,
  maxPerFile = 0,
  disabledSourceFiles = process.env.GOOGLE_BROWSER_SIDECAR_PROXY_DISABLED_SOURCES || ''
) {
  const disabled = parseSourceFileList(disabledSourceFiles);
  let dirEntries;
  try {
    dirEntries = await fs.readdir(proxyDir, { withFileTypes: true });
  } catch {
    return [];
  }
  const files = dirEntries
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
    .filter((fileName) => !disabled.has(fileName))
    .sort();

  const proxies = [];
  for (const fileName of files) {
    const filePath = path.join(proxyDir, fileName);
    const content = await fs.readFile(filePath, 'utf8');
    let countForFile = 0;
    for (const [index, line] of content.split(/\r?\n/).entries()) {
      const url = parseProxyLine(line);
      if (!url) continue;
      proxies.push({
        id: `${fileName}:${index}`,
        url,
        sourceFile: fileName,
        successCount: 0,
        failureCount: 0,
        cooldownUntil: 0,
        lastError: null,
        lastDurationMs: null,
        lastUsedAt: 0,
        lastSuccessAt: 0
      });
      countForFile += 1;
      if (maxPerFile > 0 && countForFile >= maxPerFile) break;
    }
  }
  return proxies;
}

export function parseProxyLine(line) {
  const value = line.trim();
  if (!value || value.startsWith('#')) return null;
  if (
    value.startsWith('http://') ||
    value.startsWith('https://') ||
    value.startsWith('socks5://') ||
    value.startsWith('socks5h://')
  ) {
    return value;
  }

  const parts = value.split(':');
  if (parts.length === 2) {
    const [host, port] = parts;
    return `http://${host}:${port}`;
  }
  if (parts.length === 4) {
    const [host, port, user, pass] = parts;
    return `http://${encodeURIComponent(user)}:${encodeURIComponent(pass)}@${host}:${port}`;
  }
  return null;
}

export function maskProxyUrl(proxyUrl) {
  try {
    const parsed = new URL(proxyUrl);
    const auth = parsed.username ? `${parsed.username ? '***' : ''}${parsed.password ? ':***' : ''}@` : '';
    return `${parsed.protocol}//${auth}${parsed.hostname}:${parsed.port}`;
  } catch {
    return proxyUrl;
  }
}

function truncateError(error) {
  const message = error instanceof Error ? error.message : String(error);
  return message.slice(0, 240);
}

function parseSourceWeights(value, defaultWeight) {
  const map = new Map();
  for (const item of String(value)
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)) {
    const [sourceFile, rawWeight] = item.split(':');
    const weight = Number.parseInt(String(rawWeight || defaultWeight), 10);
    if (!sourceFile || !Number.isFinite(weight)) continue;
    map.set(sourceFile, weight);
  }
  return map;
}

function parseSourceFileList(value) {
  if (value instanceof Set) return new Set(value);
  if (Array.isArray(value)) {
    return new Set(
      value
        .map((item) => String(item).trim())
        .filter(Boolean)
    );
  }
  return new Set(
    String(value)
      .split(',')
      .map((item) => item.trim())
      .filter(Boolean)
  );
}

function scoreProxyEntry(entry, sourceWeights, defaultWeight) {
  const baseWeight = sourceWeights.get(entry.sourceFile) ?? defaultWeight;
  const successBoost = entry.successCount * 20;
  const failurePenalty = entry.failureCount * 25;
  const durationPenalty =
    entry.lastDurationMs && entry.lastDurationMs > 0
      ? Math.min(20, Math.round(entry.lastDurationMs / 250))
      : 0;
  const recentSuccessBoost = entry.lastSuccessAt > 0 ? 5 : 0;
  return baseWeight + successBoost + recentSuccessBoost - failurePenalty - durationPenalty;
}

function normalizeSet(value) {
  if (value instanceof Set) return value;
  return new Set(Array.isArray(value) ? value : []);
}
