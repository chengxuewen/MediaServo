const { chromium } = require('playwright');
const { execSync } = require('child_process');
(async () => {
  const token = execSync(`docker logs mediaservo-server-1 2>&1 | grep -oE "eyJ[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+" | tail -1`).toString().trim();
  const browser = await chromium.launch({
    executablePath: '/home/maxsense/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome',
    headless: true,
    args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'],
  });
  const page = await browser.newPage();
  const logs = [];
  page.on('console', (m) => { if (['log','warn','error'].includes(m.type())) logs.push(m.text().slice(0, 160)); });
  await page.goto('http://127.0.0.1:5173/admin/login');
  await page.evaluate((t) => localStorage.setItem('mediaservo_admin_token', t), token);
  await page.goto('http://127.0.0.1:5173/admin');
  await page.waitForSelector('.device-group', { timeout: 15000 });
  await page.locator('.device-header').first().click();
  await page.waitForTimeout(500);
  await page.locator('.btn-play').first().click();
  await page.waitForTimeout(12000);
  const videos = await page.evaluate(() => Array.from(document.querySelectorAll('video')).map(v => ({ r: v.readyState, w: v.videoWidth })));
  console.log('视频:', JSON.stringify(videos));
  const key = logs.filter(l => /candidate|cand=|iceConnectionState|disconnected|connected|consum|transport created/i.test(l));
  console.log('关键(' + key.length + '):');
  key.slice(-15).forEach(l => console.log('  ' + l.slice(0, 140)));
  await browser.close();
})().catch(e => { console.error('FAIL:', e.message); process.exit(1); });
