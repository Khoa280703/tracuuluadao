import { launch as cloakLaunch } from 'cloakbrowser';
import {
  buildCloakLaunchOptions,
  loadProjectEnvIfPresent,
  parseArgs,
  readSidecarConfigFromEnv,
  runGoogleSearchJob
} from './browser-search.mjs';

loadProjectEnvIfPresent();

const argv = parseArgs(process.argv.slice(2));
const config = readSidecarConfigFromEnv();

let browser;

try {
  browser = await cloakLaunch(buildCloakLaunchOptions(config));

  const payload = await runGoogleSearchJob(
    browser,
    {
      query: argv.query || '0562015037',
      queryType: argv['query-type'] || 'phone',
      proxy: argv.proxy || null,
      limit: Number.parseInt(argv.limit || '30', 10)
    },
    config
  );

  await browser.close();
  process.stdout.write(JSON.stringify(payload));
} catch (error) {
  if (browser) await browser.close().catch(() => {});
  process.stdout.write(
    JSON.stringify({
      success: false,
      search_results: [],
      metadata: {
        mode: 'browser',
        error: error instanceof Error ? error.message : String(error)
      },
      raw_html: ''
    })
  );
  process.exit(1);
}
