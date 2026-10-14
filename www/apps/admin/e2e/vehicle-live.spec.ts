import { test, expect } from '@playwright/test';

test('vehicle live video rendering (9800 production entry)', async ({ page }) => {
  const logs: string[] = [];
  page.on('console', m => { if (/sfu|SDP|error|failed|ONTRACK|consume|producer|video/i.test(m.text())) logs.push(m.text().slice(0, 150)); });
  page.on('pageerror', e => logs.push('PAGEERROR: ' + e.message.slice(0, 200)));

  await page.goto('http://127.0.0.1:9800/admin', { waitUntil: 'networkidle', timeout: 20000 });

  // login (9800 origin has no storageState token → login form)
  const user = page.locator('input[type="text"], input[name="username"]').first();
  if (await user.count()) {
    await user.fill('admin');
    await page.locator('input[type="password"]').first().fill('admin123');
    await page.locator('button[type="submit"], button:has-text("Login")').first().click();
    await page.waitForTimeout(1500);
  }

  // Dashboard → Play(vehicle)
  await page.waitForTimeout(2500);
  const play = page.locator('.device-group', { hasText: 'vehicle' }).locator('button').filter({ hasText: /^Play$/ }).first();
  await expect(play).toBeVisible({ timeout: 15000 });
  await play.click();
  console.log('CLICKED Play(vehicle)');

  // wait for video to render
  await page.waitForFunction(() => {
    const v = document.querySelector('video');
    return v && v.readyState >= 2 && v.videoWidth > 0;
  }, { timeout: 20000 }).then(() => {
    console.log('VIDEO RENDERED');
  }).catch(async () => {
    const info = await page.evaluate(() => {
      const v = document.querySelector('video');
      return v ? { readyState: v.readyState, videoWidth: v.videoWidth, videoHeight: v.videoHeight, paused: v.paused } : null;
    });
    console.log('NO VIDEO:', JSON.stringify(info));
    console.log('--- LOGS ---');
    logs.slice(-25).forEach(l => console.log(l));
    throw new Error('video not rendered');
  });

  // success assertion
  const dims = await page.evaluate(() => {
    const v = document.querySelector('video')!;
    return { w: v.videoWidth, h: v.videoHeight, rs: v.readyState };
  });
  console.log('VIDEO OK:', JSON.stringify(dims));
});
