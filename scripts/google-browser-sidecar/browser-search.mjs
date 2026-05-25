import { readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';

export function loadProjectEnvIfPresent() {
  if (process.env.GOOGLE_BROWSER_SIDECAR_ENV_LOADED === '1') return;
  const candidates = ['.env', '../../.env'];
  for (const candidate of candidates) {
    try {
      if (!existsSync(candidate)) continue;
      const envDir = resolve(dirname(candidate));
      const lines = readFileSync(candidate, 'utf8').split('\n');
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('#')) continue;
        const eqIndex = trimmed.indexOf('=');
        if (eqIndex < 1) continue;
        const key = trimmed.slice(0, eqIndex);
        if (process.env[key] !== undefined) continue;
        let value = trimmed.slice(eqIndex + 1);
        if (key === 'PROXY_DIR' && value.startsWith('./')) {
          value = resolve(envDir, value);
        }
        process.env[key] = value;
      }
      break;
    } catch {}
  }
  process.env.GOOGLE_BROWSER_SIDECAR_ENV_LOADED = '1';
}

export function readSidecarConfigFromEnv() {
  const disableImages = process.env.GOOGLE_BROWSER_SIDECAR_DISABLE_IMAGES === '1';
  const blockedResourceTypes = (
    process.env.GOOGLE_BROWSER_SIDECAR_BLOCKED_RESOURCE_TYPES || 'media'
  )
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);

  return {
    chromePath: process.env.GOOGLE_BROWSER_SIDE_CAR_CHROME_PATH || '/usr/bin/google-chrome',
    googleMinimalMode: process.env.GOOGLE_BROWSER_SIDECAR_GOOGLE_MINIMAL_MODE !== '0',
    disableImages,
    blockedResourceTypes,
    launchArgs: buildLaunchArgs(disableImages),
    userAgent:
      process.env.GOOGLE_BROWSER_SIDECAR_USER_AGENT ||
      'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/144.0.0.0 Safari/537.36',
    locale: process.env.GOOGLE_BROWSER_SIDECAR_LOCALE || 'vi-VN',
    viewport: {
      width: Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_VIEWPORT_WIDTH || '1366', 10),
      height: Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_VIEWPORT_HEIGHT || '2200', 10)
    },
    fallbackSettleMs: Number.parseInt(
      process.env.GOOGLE_BROWSER_SIDECAR_FALLBACK_SETTLE_MS || '1200',
      10
    ),
    selectorTimeoutMs: Number.parseInt(
      process.env.GOOGLE_BROWSER_SIDECAR_SELECTOR_TIMEOUT_MS || '6000',
      10
    ),
    gotoTimeoutMs: Number.parseInt(process.env.GOOGLE_BROWSER_SIDECAR_GOTO_TIMEOUT_MS || '22000', 10),
    waitUntil: process.env.GOOGLE_BROWSER_SIDECAR_WAIT_UNTIL || 'load',
    consentTimeoutMs: Number.parseInt(
      process.env.GOOGLE_BROWSER_SIDECAR_CONSENT_TIMEOUT_MS || '2500',
      10
    ),
    postConsentSettleMs: Number.parseInt(
      process.env.GOOGLE_BROWSER_SIDECAR_POST_CONSENT_SETTLE_MS || '1500',
      10
    ),
    challengeSettleMs: Number.parseInt(
      process.env.GOOGLE_BROWSER_SIDECAR_CHALLENGE_SETTLE_MS || '1800',
      10
    ),
    fingerprintRotationEnabled: process.env.GOOGLE_BROWSER_SIDECAR_FINGERPRINT_ROTATION_ENABLED !== '0',
    acceptLanguage:
      process.env.GOOGLE_BROWSER_SIDECAR_ACCEPT_LANGUAGE || 'vi-VN,vi;q=0.9,en;q=0.8'
  };
}

export async function runGoogleSearchJob(
  browser,
  job,
  config,
  extraMetadata = {},
  proxyRuntime = {}
) {
  const proxyAttemptsPerQuery = Math.max(
    1,
    Number.parseInt(
      String(proxyRuntime.proxyAttemptsPerQuery || proxyRuntime.proxyAttemptsPerPage || 1),
      10
    )
  );
  const explicitProxy = job.proxy || proxyRuntime.explicitProxy || null;
  const seenProxyIds = new Set();
  const seenSourceFiles = new Set();
  const maxAttempts = explicitProxy ? 1 : proxyRuntime.pickProxyEntry ? proxyAttemptsPerQuery + 1 : 1;
  let lastPayload = null;

  for (let attemptIndex = 0; attemptIndex < maxAttempts; attemptIndex += 1) {
    const proxyChoice = chooseQueryProxy(
      explicitProxy,
      proxyRuntime,
      attemptIndex,
      seenProxyIds,
      seenSourceFiles
    );
    if (attemptIndex > 0 && !proxyChoice) {
      break;
    }

    const payload = await runSingleQueryAttempt(
      browser,
      { ...job, attemptIndex },
      config,
      {
        ...extraMetadata,
        proxy_source_file: proxyChoice?.sourceFile || null,
        proxy_masked_url: proxyChoice?.maskedUrl || maskProxyUrlSafe(explicitProxy)
      },
      proxyChoice?.url || explicitProxy || null
    );

    const attemptRecord = {
      attempt_index: attemptIndex + 1,
      proxy_source_file: proxyChoice?.sourceFile || null,
      proxy_masked_url: proxyChoice?.maskedUrl || maskProxyUrlSafe(explicitProxy),
      fingerprint_id: payload.metadata?.fingerprint_id || null,
      fingerprint_label: payload.metadata?.fingerprint_label || null,
      success: payload.success,
      transport_status: payload.metadata?.transport_status || 'unknown',
      google_status: payload.metadata?.google_status || 'unknown',
      result_count: payload.metadata?.result_count || 0,
      pages_attempted: payload.metadata?.pages_attempted || 0,
      pages_succeeded: payload.metadata?.pages_succeeded || 0,
      duration_ms: payload.metadata?.total_duration_ms || null
    };
    proxyRuntime.onAttempt?.(attemptRecord);

    if (proxyChoice?.entry) {
      if (payload.success) {
        proxyRuntime.markProxySuccess?.(proxyChoice.entry, {
          durationMs: payload.metadata?.total_duration_ms || null
        });
      } else {
        seenSourceFiles.add(proxyChoice.entry.sourceFile);
        proxyRuntime.markProxyFailure?.(
          proxyChoice.entry,
          `query ${payload.metadata?.google_status || payload.metadata?.transport_status || 'failed'}`
        );
      }
    }

    lastPayload = payload;
    if (payload.success || !shouldRetryQueryWithAnotherProxy(payload)) {
      return payload;
    }
  }

  return (
    lastPayload || {
      success: false,
      search_results: [],
      metadata: {
        mode: 'browser',
        transport_status: 'proxy_error',
        google_status: 'unknown',
        content_status: 'unknown',
        result_count: 0,
        ...extraMetadata
      },
      raw_html: ''
    }
  );
}

function chooseQueryProxy(explicitProxy, proxyRuntime, attemptIndex, seenProxyIds, seenSourceFiles) {
  if (explicitProxy) {
    return {
      url: explicitProxy,
      entry: null,
      sourceFile: null,
      maskedUrl: maskProxyUrlSafe(explicitProxy)
    };
  }
  if (attemptIndex === 0) {
    return null;
  }

  let entry =
    proxyRuntime.pickProxyEntry?.(seenProxyIds, seenSourceFiles) ||
    proxyRuntime.pickProxyEntry?.(seenProxyIds, new Set()) ||
    null;
  if (!entry) return null;
  seenProxyIds.add(entry.id);
  return {
    url: entry.url,
    entry,
    sourceFile: entry.sourceFile,
    maskedUrl: maskProxyUrlSafe(entry.url)
  };
}

async function runSingleQueryAttempt(browser, job, config, extraMetadata, proxyUrl) {
  if (config.googleMinimalMode && !proxyUrl) {
    return runSingleQueryAttemptMinimal(browser, job, config, extraMetadata);
  }
  return runSingleQueryAttemptWithContext(browser, job, config, extraMetadata, proxyUrl);
}

async function runSingleQueryAttemptMinimal(browser, job, config, extraMetadata) {
  const startedAt = Date.now();
  const limit = Number.parseInt(String(job.limit || 30), 10);
  const pageSize = Math.min(limit, 10);
  const totalPages = Math.max(1, Math.ceil(limit / pageSize));
  const metadata = {
    mode: 'browser',
    proxy_used: false,
    disable_images: false,
    blocked_resource_types: [],
    query_attempted: job.query,
    query_duration_ms: 0,
    resources_aborted: 0,
    resources_allowed: 0,
    transport_status: 'unknown',
    google_status: 'unknown',
    content_status: 'unknown',
    result_count: 0,
    pages_attempted: 0,
    pages_succeeded: 0,
    fingerprint_id: 'minimal-default',
    fingerprint_label: 'cloakbrowser-default-page',
    ...extraMetadata
  };
  const aggregated = [];
  const rawSamples = [];
  const seenUrls = new Set();
  let lastPageFailure = null;
  const queryStartedAt = Date.now();
  const page = await browser.newPage();

  try {
    for (let pageIndex = 0; pageIndex < totalPages && aggregated.length < limit; pageIndex += 1) {
      const pageStart = pageIndex * pageSize;
      metadata.pages_attempted += 1;

      const pageAttempt = await runSingleSearchPageMinimal(
        page,
        { ...job, pageIndex, pageStart, pageSize },
        config
      );

      if (pageAttempt.rawSample && rawSamples.length < 2) {
        rawSamples.push(pageAttempt.rawSample);
      }

      if (pageAttempt.payload?.metadata?.transport_status === 'ok') {
        metadata.transport_status = 'ok';
      } else if (metadata.transport_status === 'unknown') {
        metadata.transport_status = pageAttempt.payload?.metadata?.transport_status || 'unknown';
      }

      if (!pageAttempt.succeeded) {
        lastPageFailure = pageAttempt;
        if (aggregated.length === 0) {
          metadata.google_status = pageAttempt.payload?.metadata?.google_status || 'unknown';
          metadata.content_status = 'unknown';
        }
        break;
      }

      metadata.pages_succeeded += 1;
      for (const result of pageAttempt.payload.search_results) {
        if (seenUrls.has(result.url)) continue;
        seenUrls.add(result.url);
        aggregated.push(result);
        if (aggregated.length >= limit) break;
      }
    }

    metadata.query_duration_ms = Date.now() - queryStartedAt;
    metadata.total_duration_ms = Date.now() - startedAt;
    metadata.result_count = aggregated.length;

    if (aggregated.length > 0) {
      metadata.google_status = 'serp';
      metadata.content_status = 'has_results';
      metadata.transport_status = metadata.transport_status === 'unknown' ? 'ok' : metadata.transport_status;
    } else if (metadata.google_status === 'unknown') {
      metadata.google_status = lastPageFailure?.payload?.metadata?.google_status || 'unknown';
      metadata.content_status = metadata.google_status === 'serp' ? 'empty_results' : 'unknown';
    } else if (metadata.google_status === 'serp') {
      metadata.content_status = 'empty_results';
    }

    return {
      success: metadata.transport_status === 'ok' && metadata.google_status === 'serp',
      search_results: aggregated,
      metadata,
      raw_html: rawSamples.join('\n\n---\n\n')
    };
  } finally {
    await page.close().catch(() => {});
  }
}

async function runSingleQueryAttemptWithContext(browser, job, config, extraMetadata, proxyUrl) {
  const startedAt = Date.now();
  const limit = Number.parseInt(String(job.limit || 30), 10);
  const pageSize = Math.min(limit, 10);
  const totalPages = Math.max(1, Math.ceil(limit / pageSize));
  const fingerprint = pickFingerprintProfile(
    config,
    { ...job, pageIndex: 0, pageStart: 0 },
    proxyUrl
  );
  const metadata = {
    mode: 'browser',
    proxy_used: Boolean(proxyUrl),
    disable_images: config.disableImages,
    blocked_resource_types: config.blockedResourceTypes,
    query_attempted: job.query,
    query_duration_ms: 0,
    resources_aborted: 0,
    resources_allowed: 0,
    transport_status: 'unknown',
    google_status: 'unknown',
    content_status: 'unknown',
    result_count: 0,
    pages_attempted: 0,
    pages_succeeded: 0,
    fingerprint_id: fingerprint.id,
    fingerprint_label: fingerprint.label,
    ...extraMetadata
  };
  const context = await browser.newContext({
    locale: fingerprint.locale,
    userAgent: fingerprint.userAgent,
    viewport: fingerprint.viewport,
    timezoneId: fingerprint.timezoneId,
    colorScheme: fingerprint.colorScheme,
    deviceScaleFactor: fingerprint.deviceScaleFactor,
    extraHTTPHeaders: {
      'Accept-Language': fingerprint.acceptLanguage
    },
    ...(proxyUrl ? { proxy: toPlaywrightProxy(proxyUrl) } : {})
  });

  await context.route('**/*', (route) => {
    const resourceType = route.request().resourceType();
    if (config.blockedResourceTypes.includes(resourceType)) {
      metadata.resources_aborted += 1;
      return route.abort();
    }
    metadata.resources_allowed += 1;
    return route.continue();
  });

  const aggregated = [];
  const rawSamples = [];
  const seenUrls = new Set();
  let lastPageFailure = null;
  const queryStartedAt = Date.now();

  try {
    for (let pageIndex = 0; pageIndex < totalPages && aggregated.length < limit; pageIndex += 1) {
      const pageStart = pageIndex * pageSize;
      metadata.pages_attempted += 1;

      const pageAttempt = await runSingleSearchPage(
        context,
        { ...job, pageIndex, pageStart, pageSize },
        config
      );

      if (pageAttempt.rawSample && rawSamples.length < 2) {
        rawSamples.push(pageAttempt.rawSample);
      }

      if (pageAttempt.payload?.metadata?.transport_status === 'ok') {
        metadata.transport_status = 'ok';
      } else if (metadata.transport_status === 'unknown') {
        metadata.transport_status = pageAttempt.payload?.metadata?.transport_status || 'unknown';
      }

      if (!pageAttempt.succeeded) {
        lastPageFailure = pageAttempt;
        if (aggregated.length === 0) {
          metadata.google_status = pageAttempt.payload?.metadata?.google_status || 'unknown';
          metadata.content_status = 'unknown';
        }
        break;
      }

      metadata.pages_succeeded += 1;
      for (const result of pageAttempt.payload.search_results) {
        if (seenUrls.has(result.url)) continue;
        seenUrls.add(result.url);
        aggregated.push(result);
        if (aggregated.length >= limit) break;
      }
    }

    metadata.query_duration_ms = Date.now() - queryStartedAt;
    metadata.total_duration_ms = Date.now() - startedAt;
    metadata.result_count = aggregated.length;

    if (aggregated.length > 0) {
      metadata.google_status = 'serp';
      metadata.content_status = 'has_results';
      metadata.transport_status = metadata.transport_status === 'unknown' ? 'ok' : metadata.transport_status;
    } else if (metadata.google_status === 'unknown') {
      metadata.google_status = lastPageFailure?.payload?.metadata?.google_status || 'unknown';
      metadata.content_status = metadata.google_status === 'serp' ? 'empty_results' : 'unknown';
    } else if (metadata.google_status === 'serp') {
      metadata.content_status = 'empty_results';
    }

    return {
      success: metadata.transport_status === 'ok' && metadata.google_status === 'serp',
      search_results: aggregated,
      metadata,
      raw_html: rawSamples.join('\n\n---\n\n')
    };
  } finally {
    await context.close().catch(() => {});
  }
}

async function runSingleSearchPage(context, job, config) {
  const startedAt = Date.now();
  const metadata = {
    transport_status: 'unknown',
    google_status: 'unknown',
    content_status: 'unknown',
    result_count: 0,
    query_duration_ms: 0,
    total_duration_ms: 0
  };
  let rawHtml = '';
  const page = await context.newPage();
  const queryStartedAt = Date.now();
  try {
    const searchUrl = new URL('https://www.google.com/search');
    searchUrl.searchParams.set('q', job.query);
    searchUrl.searchParams.set('hl', 'vi');
    searchUrl.searchParams.set('gl', 'vn');
    searchUrl.searchParams.set('num', String(job.pageSize));
    searchUrl.searchParams.set('start', String(job.pageStart));

    await page.goto(searchUrl.toString(), {
      waitUntil: config.waitUntil,
      timeout: config.gotoTimeoutMs
    });
    metadata.transport_status = 'ok';
    await waitForSearchSignals(page, config.selectorTimeoutMs);
    await dismissConsentIfPresent(page, config);
    await waitForSearchSignals(page, config.selectorTimeoutMs);
    if (isConsentPage(page.url(), '')) {
      await page.waitForTimeout(config.challengeSettleMs);
      await dismissConsentIfPresent(page, config);
      await waitForSearchSignals(page, config.selectorTimeoutMs);
    }
    await page.waitForTimeout(config.fallbackSettleMs);

    rawHtml = await page.content();
    const results = await page.evaluate((cap) => {
      const snippetSelectors = [
        'div.VwiC3b',
        "div[data-sncf='1']",
        'span.aCOpRe',
        'div.s3v9rd',
        'div.yXK7lf',
        'div.BNeawe.s3v9rd',
        'div.uhHOwf.BYbUcd'
      ];
      const out = [];
      const pageSeenUrls = new Set();

      for (const heading of document.querySelectorAll('h3')) {
        const title = heading.textContent?.trim();
        if (!title) continue;
        const anchor = heading.closest('a[href]');
        const href = anchor?.getAttribute('href') || '';
        if (!href) continue;

        let url = href;
        if (href.startsWith('/url?')) {
          try {
            const wrapped = new URL(`https://www.google.com${href}`);
            url = wrapped.searchParams.get('q') || href;
          } catch {}
        }

        if (!/^https?:\/\//i.test(url) || pageSeenUrls.has(url)) continue;
        pageSeenUrls.add(url);

        let snippet = '';
        let current = heading.parentElement;
        while (current && !snippet) {
          for (const selector of snippetSelectors) {
            const node = current.querySelector(selector);
            if (node?.textContent?.trim()) {
              snippet = node.textContent.trim();
              break;
            }
          }
          current = current.parentElement;
        }

        out.push({ title, url, snippet });
        if (out.length >= cap) break;
      }
      return out;
    }, job.pageSize);

    metadata.query_duration_ms = Date.now() - queryStartedAt;
    metadata.result_count = results.length;
    const blockKind = detectGoogleBlockKind(page.url(), rawHtml, results.length);
    if (blockKind) {
      metadata.google_status = blockKind;
      metadata.content_status = 'unknown';
    } else {
      metadata.google_status = 'serp';
      metadata.content_status = results.length > 0 ? 'has_results' : 'empty_results';
    }
    return {
      success: metadata.transport_status === 'ok' && metadata.google_status === 'serp',
      search_results: results,
      metadata: {
        ...metadata,
        total_duration_ms: Date.now() - startedAt
      },
      raw_html: rawHtml
    };
  } catch (error) {
    metadata.query_duration_ms = Date.now() - queryStartedAt;
    metadata.transport_status = classifyTransportError(error);
    metadata.google_status = 'unknown';
    metadata.content_status = 'unknown';
    return {
      success: false,
      search_results: [],
      metadata: {
        ...metadata,
        total_duration_ms: Date.now() - startedAt
      },
      raw_html: rawHtml
    };
  } finally {
    await page.close().catch(() => {});
  }
}

async function runSingleSearchPageMinimal(page, job, config) {
  const startedAt = Date.now();
  const metadata = {
    transport_status: 'unknown',
    google_status: 'unknown',
    content_status: 'unknown',
    result_count: 0,
    query_duration_ms: 0,
    total_duration_ms: 0
  };
  let rawHtml = '';
  const queryStartedAt = Date.now();
  try {
    const searchUrl = new URL('https://www.google.com/search');
    searchUrl.searchParams.set('q', job.query);
    searchUrl.searchParams.set('hl', 'vi');
    searchUrl.searchParams.set('gl', 'vn');
    searchUrl.searchParams.set('num', String(job.pageSize));
    searchUrl.searchParams.set('start', String(job.pageStart));

    await page.goto(searchUrl.toString(), {
      waitUntil: 'load',
      timeout: Math.max(config.gotoTimeoutMs, 30000)
    });
    metadata.transport_status = 'ok';
    await page.waitForTimeout(Math.max(config.fallbackSettleMs, 4000));

    rawHtml = await page.content();
    const results = await page.evaluate((cap) => {
      const out = [];
      const pageSeenUrls = new Set();
      for (const heading of document.querySelectorAll('h3')) {
        const title = heading.textContent?.trim();
        if (!title) continue;
        const anchor = heading.closest('a[href]');
        const href = anchor?.getAttribute('href') || '';
        if (!href) continue;

        let url = href;
        if (href.startsWith('/url?')) {
          try {
            const wrapped = new URL(`https://www.google.com${href}`);
            url = wrapped.searchParams.get('q') || href;
          } catch {}
        }

        if (!/^https?:\/\//i.test(url) || pageSeenUrls.has(url)) continue;
        pageSeenUrls.add(url);
        out.push({
          title,
          url,
          snippet: ''
        });
        if (out.length >= cap) break;
      }
      return out;
    }, job.pageSize);

    metadata.query_duration_ms = Date.now() - queryStartedAt;
    metadata.result_count = results.length;
    const blockKind = detectGoogleBlockKind(page.url(), rawHtml, results.length);
    if (blockKind) {
      metadata.google_status = blockKind;
      metadata.content_status = 'unknown';
    } else {
      metadata.google_status = 'serp';
      metadata.content_status = results.length > 0 ? 'has_results' : 'empty_results';
    }
    const payload = {
      success: metadata.transport_status === 'ok' && metadata.google_status === 'serp',
      search_results: results,
      metadata: {
        ...metadata,
        total_duration_ms: Date.now() - startedAt
      },
      raw_html: rawHtml
    };
    return {
      succeeded: payload.success || payload.metadata.google_status === 'serp',
      payload,
      rawSample: payload.raw_html ? payload.raw_html.slice(0, 2000) : ''
    };
  } catch (error) {
    metadata.query_duration_ms = Date.now() - queryStartedAt;
    metadata.transport_status = classifyTransportError(error);
    metadata.google_status = 'unknown';
    metadata.content_status = 'unknown';
    const payload = {
      success: false,
      search_results: [],
      metadata: {
        ...metadata,
        total_duration_ms: Date.now() - startedAt
      },
      raw_html: rawHtml
    };
    return {
      succeeded: false,
      payload,
      rawSample: payload.raw_html ? payload.raw_html.slice(0, 2000) : ''
    };
  }
}

function pickFingerprintProfile(config, job, proxyUrl) {
  if (!config.fingerprintRotationEnabled) {
    return {
      id: 'static-default',
      label: 'linux-chrome-static',
      userAgent: config.userAgent,
      viewport: config.viewport,
      timezoneId: 'Asia/Ho_Chi_Minh',
      locale: config.locale,
      acceptLanguage: config.acceptLanguage,
      colorScheme: 'light',
      deviceScaleFactor: 1
    };
  }

  const profiles = buildFingerprintProfiles(config);
  const seed = stableHash(
    `${job.query}|${job.pageIndex}|${job.pageStart}|${job.attemptIndex || 0}|${proxyUrl || 'direct'}`
  );
  return profiles[seed % profiles.length];
}

function buildFingerprintProfiles(config) {
  const acceptLanguages = [
    config.acceptLanguage,
    'vi-VN,vi;q=0.95,en-US;q=0.8,en;q=0.7',
    'vi-VN,vi;q=0.9,en-US;q=0.75,en;q=0.65'
  ];
  const locales = ['vi-VN', 'vi-VN', 'en-US', 'en-GB'];
  const timezones = ['Asia/Ho_Chi_Minh', 'Asia/Bangkok', 'Asia/Ho_Chi_Minh', 'Asia/Singapore'];
  const palettes = [
    {
      label: 'win-chrome-136',
      userAgent:
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36',
      viewport: { width: 1365, height: 2140 },
      colorScheme: 'light',
      deviceScaleFactor: 1
    },
    {
      label: 'linux-chrome-135',
      userAgent:
        'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36',
      viewport: { width: 1440, height: 2200 },
      colorScheme: 'light',
      deviceScaleFactor: 1
    },
    {
      label: 'mac-chrome-136',
      userAgent:
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36',
      viewport: { width: 1512, height: 2100 },
      colorScheme: 'light',
      deviceScaleFactor: 2
    },
    {
      label: 'win-edge-136',
      userAgent:
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0',
      viewport: { width: 1600, height: 2260 },
      colorScheme: 'light',
      deviceScaleFactor: 1
    }
  ];

  return palettes.map((profile, index) => ({
    id: `fp-${index + 1}`,
    label: profile.label,
    userAgent: profile.userAgent,
    viewport: profile.viewport,
    colorScheme: profile.colorScheme,
    deviceScaleFactor: profile.deviceScaleFactor,
    locale: locales[index % locales.length],
    acceptLanguage: acceptLanguages[index % acceptLanguages.length],
    timezoneId: timezones[index % timezones.length]
  }));
}

function stableHash(input) {
  let hash = 2166136261;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash >>> 0);
}

function shouldRetryQueryWithAnotherProxy(payload) {
  const transportStatus = payload.metadata?.transport_status;
  const googleStatus = payload.metadata?.google_status;
  return (
    transportStatus !== 'ok' ||
    googleStatus === 'unknown' ||
    googleStatus === 'captcha' ||
    googleStatus === 'enablejs' ||
    googleStatus === 'blocked'
  );
}

function maskProxyUrlSafe(proxyUrl) {
  if (!proxyUrl) return null;
  try {
    const parsed = new URL(proxyUrl);
    const auth = parsed.username ? `${parsed.username ? '***' : ''}${parsed.password ? ':***' : ''}@` : '';
    return `${parsed.protocol}//${auth}${parsed.hostname}:${parsed.port}`;
  } catch {
    return proxyUrl;
  }
}

export function buildQueryCandidates(query, queryType) {
  const candidates =
    queryType === 'phone'
      ? [query, `"${query}"`, `${query} lừa đảo`, `"${query}" lừa đảo`]
      : queryType === 'bank'
        ? [query, `"${query}"`, `"${query}" tài khoản ngân hàng lừa đảo`]
        : [query, `"${query}"`, `"${query}" cảnh báo lừa đảo`];
  return [...new Set(candidates)];
}

export function buildLaunchArgs(disableImages) {
  const args = [
    '--headless=new',
    '--disable-blink-features=AutomationControlled',
    '--disable-dev-shm-usage',
    '--no-sandbox',
    '--disable-setuid-sandbox',
    '--mute-audio',
    '--no-first-run',
    '--no-default-browser-check',
    '--password-store=basic',
    '--use-mock-keychain'
  ];

  if (disableImages) {
    args.push('--blink-settings=imagesEnabled=false');
  }

  return args;
}

export function buildCloakLaunchOptions(config, options = {}) {
  const disableImages = options.disableImages ?? config.disableImages;
  const minimalMode = options.minimalMode ?? config.googleMinimalMode;
  return {
    headless: true,
    timezone: 'Asia/Ho_Chi_Minh',
    locale: 'vi-VN',
    ...(minimalMode ? {} : { args: buildLaunchArgs(disableImages) })
  };
}

export function isBlocked(currentUrl, body, resultCount) {
  return detectGoogleBlockKind(currentUrl, body, resultCount) !== null;
}

export function detectGoogleBlockKind(currentUrl, body, resultCount) {
  const lower = `${currentUrl}\n${body}`.toLowerCase();
  const hardBlocks = [
    ['captcha', 'detected unusual traffic'],
    ['captcha', 'g-recaptcha'],
    ['captcha', '/sorry/index'],
    ['captcha', 'id="captcha"'],
    ['captcha', 'name="captcha"'],
    ['captcha', 'captcha-form'],
    ['blocked', 'consent.google']
  ];
  for (const [kind, needle] of hardBlocks) {
    if (lower.includes(needle)) return kind;
  }
  if (resultCount > 0) return null;
  if (lower.includes('/httpservice/retry/enablejs')) return 'enablejs';
  return null;
}

export async function dismissConsentIfPresent(page) {
  for (const label of ['Tôi đồng ý', 'I agree', 'Accept all', 'Chấp nhận tất cả']) {
    const button = page.getByRole('button', { name: label }).first();
    if (await button.isVisible().catch(() => false)) {
      await button.click({ timeout: 2000 }).catch(() => {});
      await page.waitForTimeout(1200);
      return;
    }
  }
}

async function waitForSearchSignals(page, timeoutMs) {
  await Promise.race([
    page.waitForSelector('h3', { timeout: timeoutMs }).catch(() => null),
    page.waitForSelector('textarea[name="q"]', { timeout: timeoutMs }).catch(() => null),
    page.waitForSelector('form[action*="consent"]', { timeout: timeoutMs }).catch(() => null),
    page.waitForSelector('button', { timeout: timeoutMs }).catch(() => null)
  ]);
}

function isConsentPage(currentUrl, body) {
  const lower = `${currentUrl}\n${body}`.toLowerCase();
  return lower.includes('consent.google');
}

export function toPlaywrightProxy(proxyUrl) {
  const parsed = new URL(proxyUrl);
  return {
    server: `${parsed.protocol}//${parsed.hostname}:${parsed.port}`,
    username: decodeURIComponent(parsed.username || ''),
    password: decodeURIComponent(parsed.password || '')
  };
}

export function classifyTransportError(error) {
  const message = error instanceof Error ? error.message.toLowerCase() : String(error).toLowerCase();
  if (message.includes('timeout')) return 'timeout';
  if (message.includes('proxy')) return 'proxy_error';
  return 'browser_error';
}

export function parseArgs(args) {
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
