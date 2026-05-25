import { launch as cloakLaunch } from 'cloakbrowser';

const QUERY = 'lừa đảo qua mạng';
const USER_AGENT = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36';

async function testPage(browser, pageNum, start) {
  const context = await browser.newContext({
    userAgent: USER_AGENT,
    locale: 'vi-VN',
    viewport: { width: 1920, height: 1080 },
    timezoneId: 'Asia/Ho_Chi_Minh',
  });

  const page = await context.newPage();
  try {
    const url = `https://www.google.com/search?q=${encodeURIComponent(QUERY)}&start=${start}&hl=vi&gl=vn`;
    console.log(`\n--- Page ${pageNum} (start=${start}) ---`);

    const resp = await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
    console.log(`Status: ${resp.status()}`);

    await page.waitForTimeout(2000);

    const results = await page.evaluate(() => {
      const items = document.querySelectorAll('div.g, div[data-hveid] h3');
      const captcha = document.querySelector('#captcha-form, form[action*="sorry"]');
      const enableJs = document.body?.innerText?.includes('enable JavaScript');
      return {
        resultCount: items.length,
        hasCaptcha: !!captcha,
        hasEnableJs: !!enableJs,
        title: document.title,
        bodySnippet: document.body?.innerText?.substring(0, 300) || '',
      };
    });

    console.log(`Title: ${results.title}`);
    console.log(`Results: ${results.resultCount}`);
    console.log(`Captcha: ${results.hasCaptcha} | EnableJS: ${results.hasEnableJs}`);
    if (results.resultCount === 0) {
      console.log(`Body: ${results.bodySnippet}`);
    }
    return results;
  } finally {
    await page.close();
    await context.close();
  }
}

async function main() {
  console.log('Launching CloakBrowser (no proxy, images disabled)...');
  const browser = await cloakLaunch({
    headless: true,
    args: [
      '--no-sandbox',
      '--disable-setuid-sandbox',
      '--blink-settings=imagesEnabled=false',
    ],
  });

  try {
    for (const [pageNum, start] of [[1, 0], [2, 10], [3, 20]]) {
      const r = await testPage(browser, pageNum, start);
      if (r.hasCaptcha || r.hasEnableJs) {
        console.log(`\n⚠ Blocked at page ${pageNum}, stopping.`);
        break;
      }
      await new Promise(res => setTimeout(res, 2000));
    }
  } finally {
    await browser.close();
    console.log('\nDone.');
  }
}

main().catch(err => { console.error(err); process.exit(1); });
