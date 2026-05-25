import { launch as cloakLaunch } from 'cloakbrowser';
import { readSidecarConfigFromEnv, runGoogleSearchJob, buildCloakLaunchOptions } from './browser-search.mjs';
import { ProxyRegistry, maskProxyUrl } from './proxy-registry.mjs';
import { runCheckscamSearchJob, runCheckscamDetailJob } from './jobs/checkscam.mjs';

export class BrowserPool {
  constructor(options = {}) {
    this.config = options.config || readSidecarConfigFromEnv();
    this.browserCount = Number.parseInt(String(options.browserCount || 1), 10);
    this.slotsPerBrowser = Number.parseInt(String(options.slotsPerBrowser || 1), 10);
    this.checkscamBrowserCount = Number.parseInt(String(options.checkscamBrowserCount || 1), 10);
    this.proxyRegistry = options.proxyRegistry || buildProxyRegistry(options);
    this.proxyAttemptsPerJob = Number.parseInt(
      String(options.proxyAttemptsPerJob || process.env.GOOGLE_BROWSER_SIDECAR_PROXY_ATTEMPTS_PER_JOB || 3),
      10
    );
    this.workers = [];
    this.waiters = [];
    this.checkscamWorkers = [];
    this.checkscamWaiters = [];
    this.started = false;
  }

  async start() {
    if (this.started) return;
    if (this.proxyRegistry) {
      await this.proxyRegistry.start();
    }
    this.generalBrowser = await this.launchBrowser(this.config.disableImages);
    for (let i = 0; i < this.checkscamBrowserCount; i += 1) {
      const browser = await this.launchBrowser(false);
      this.checkscamWorkers.push({ id: i, browser, active: 0, capacity: 1 });
    }
    for (let i = 0; i < this.browserCount; i += 1) {
      const browser = await this.launchBrowser(this.config.disableImages);
      this.workers.push({ id: i, browser, active: 0, capacity: this.slotsPerBrowser });
    }
    this.started = true;
  }

  async execute(job) {
    if (job.type === 'checkscam_search' || job.type === 'checkscam_detail') {
      return this.executeCheckscam(job);
    }
    const worker = await this.acquireWorker();
    try {
      const attempts = [];
      let payload;
      try {
        payload = await this.runJobOnWorker(worker, job, attempts);
      } catch (error) {
        if (!shouldRecoverClosedBrowser(error)) {
          throw error;
        }
        await this.restartWorkerBrowser(worker, true);
        payload = await this.runJobOnWorker(worker, job, attempts);
      }
      if (payload.metadata) {
        payload.metadata.proxy_attempts = attempts;
      }
      return payload;
    } finally {
      this.releaseWorker(worker);
    }
  }

  async executeCheckscam(job) {
    const worker = await this.acquireCheckscamWorker();
    try {
      const proxyRuntime = buildCheckscamProxyRuntime(this.proxyRegistry);
      try {
        if (job.type === 'checkscam_detail') {
          return await runCheckscamDetailJob(worker.browser, job, proxyRuntime);
        }
        return await runCheckscamSearchJob(worker.browser, job, proxyRuntime);
      } catch (error) {
        if (!shouldRecoverClosedBrowser(error)) throw error;
        await this.restartWorkerBrowser(worker, false);
        if (job.type === 'checkscam_detail') {
          return await runCheckscamDetailJob(worker.browser, job, proxyRuntime);
        }
        return await runCheckscamSearchJob(worker.browser, job, proxyRuntime);
      }
    } finally {
      this.releaseCheckscamWorker(worker);
    }
  }

  snapshot() {
    return {
      google_browser_count: this.browserCount,
      slots_per_browser: this.slotsPerBrowser,
      google_capacity: this.browserCount * this.slotsPerBrowser,
      google_active: this.workers.reduce((sum, w) => sum + w.active, 0),
      google_queue: this.waiters.length,
      checkscam_browser_count: this.checkscamBrowserCount,
      checkscam_active: this.checkscamWorkers.reduce((sum, w) => sum + w.active, 0),
      checkscam_queue: this.checkscamWaiters.length,
      general_browser_connected: this.generalBrowser?.isConnected?.() ?? false,
      proxy_registry: this.proxyRegistry?.snapshot() || null,
      google_workers: this.workers.map((w) => ({
        id: w.id, active: w.active, capacity: w.capacity,
        connected: w.browser?.isConnected?.() ?? false
      })),
      checkscam_workers: this.checkscamWorkers.map((w) => ({
        id: w.id, active: w.active,
        connected: w.browser?.isConnected?.() ?? false
      }))
    };
  }

  async close() {
    await this.generalBrowser?.close().catch(() => {});
    await Promise.all(this.checkscamWorkers.map((w) => w.browser.close().catch(() => {})));
    await Promise.all(this.workers.map((w) => w.browser.close().catch(() => {})));
    this.workers = [];
    this.waiters = [];
    this.checkscamWorkers = [];
    this.checkscamWaiters = [];
    this.started = false;
  }

  async acquireWorker() {
    const worker = findAvailable(this.workers);
    if (worker) { worker.active += 1; return worker; }
    return new Promise((resolve) => { this.waiters.push(resolve); });
  }

  releaseWorker(worker) {
    const waiter = this.waiters.shift();
    if (waiter) { waiter(worker); return; }
    worker.active -= 1;
  }

  async acquireCheckscamWorker() {
    const worker = findAvailable(this.checkscamWorkers);
    if (worker) { worker.active += 1; return worker; }
    return new Promise((resolve) => { this.checkscamWaiters.push(resolve); });
  }

  releaseCheckscamWorker(worker) {
    const waiter = this.checkscamWaiters.shift();
    if (waiter) { waiter(worker); return; }
    worker.active -= 1;
  }

  async launchBrowser(disableImages = this.config.disableImages) {
    return cloakLaunch(
      buildCloakLaunchOptions(this.config, {
        disableImages,
        minimalMode: disableImages === false ? false : this.config.googleMinimalMode
      })
    );
  }

  async restartWorkerBrowser(worker, disableImages = true) {
    await worker.browser?.close().catch(() => {});
    worker.browser = await this.launchBrowser(disableImages);
  }

  async runJobOnWorker(worker, job, attempts) {
    return runGoogleSearchJob(
      worker.browser,
      job,
      this.config,
      {
        worker_id: worker.id,
        browser_pool_size: this.browserCount,
        browser_slots_per_worker: this.slotsPerBrowser
      },
      buildPageProxyRuntime(job, this.proxyRegistry, this.proxyAttemptsPerJob, attempts)
    );
  }
}

function findAvailable(workers) {
  const candidates = workers
    .filter((w) => w.active < w.capacity)
    .sort((a, b) => a.active - b.active || a.id - b.id);
  return candidates.length > 0 ? candidates[0] : null;
}

function buildProxyRegistry(options) {
  const enabled =
    options.proxyRegistryEnabled ??
    process.env.GOOGLE_BROWSER_SIDECAR_PROXY_REGISTRY_ENABLED === '1';
  if (!enabled) return null;
  return new ProxyRegistry({
    proxyDir: options.proxyDir || process.env.PROXY_DIR || './proxies',
    cooldownMs: options.proxyCooldownMs,
    maxPerFile: options.proxyMaxPerFile
  });
}

function shouldRecoverClosedBrowser(error) {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes('Target page, context or browser has been closed') ||
    message.includes('Browser has been closed') ||
    message.includes('browser disconnected') ||
    message.includes('Target closed')
  );
}

function buildPageProxyRuntime(job, proxyRegistry, proxyAttemptsPerPage, attempts) {
  if (job.proxy) {
    return {
      explicitProxy: job.proxy,
      proxyAttemptsPerPage: 1,
      onAttempt: (attempt) => attempts.push(attempt)
    };
  }

  if (!proxyRegistry) {
    return {
      proxyAttemptsPerPage: 1,
      onAttempt: (attempt) => attempts.push(attempt)
    };
  }

  return {
    proxyAttemptsPerPage,
    pickProxyEntry(excludedProxyIds = new Set(), excludedSourceFiles = new Set()) {
      return pickProxyEntry(proxyRegistry, excludedProxyIds, excludedSourceFiles);
    },
    markProxySuccess(proxyEntry, metadata = {}) {
      proxyRegistry.markSuccess(proxyEntry, metadata);
    },
    markProxyFailure(proxyEntry, error) {
      proxyRegistry.markFailure(proxyEntry, error);
    },
    onAttempt(attempt) {
      attempts.push(attempt);
    }
  };
}

function pickProxyEntry(proxyRegistry, excludedProxyIds, excludedSourceFiles = new Set()) {
  if (!proxyRegistry?.entries?.length) return null;
  const excluded = excludedProxyIds instanceof Set ? excludedProxyIds : new Set(excludedProxyIds);
  const excludedSources =
    excludedSourceFiles instanceof Set ? excludedSourceFiles : new Set(excludedSourceFiles);
  for (let index = 0; index < proxyRegistry.entries.length; index += 1) {
    const proxyEntry = proxyRegistry.pick({
      excludedIds: excluded,
      excludedSourceFiles: excludedSources
    });
    if (!proxyEntry) return null;
    return proxyEntry;
  }
  return null;
}

function buildCheckscamProxyRuntime(proxyRegistry) {
  if (!proxyRegistry) return null;
  return {
    pickProxyEntry(excludedIds = new Set(), excludedSources = new Set()) {
      return pickProxyEntry(proxyRegistry, excludedIds, excludedSources);
    },
    markProxySuccess(proxyEntry, metadata = {}) {
      proxyRegistry.markSuccess(proxyEntry, metadata);
    },
    markProxyFailure(proxyEntry, error) {
      proxyRegistry.markFailure(proxyEntry, error);
    }
  };
}
