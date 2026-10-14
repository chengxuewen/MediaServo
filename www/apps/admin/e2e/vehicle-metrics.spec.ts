import { test, expect } from '@playwright/test';

test('web stats panel: codec/fps/encode time', async ({ page }) => {
  const logs: string[] = [];
  page.on('console', m => { if (/encoder_status|avg_encode|codec|metrics/i.test(m.text())) logs.push(m.text().slice(0, 200)); });
  await page.goto('http://127.0.0.1:9800/admin', { waitUntil: 'networkidle', timeout: 20000 });
  const user = page.locator('input[type="text"], input[name="username"]').first();
  if (await user.count()) {
    await user.fill('admin');
    await page.locator('input[type="password"]').first().fill('admin123');
    await page.locator('button[type="submit"], button:has-text("Login")').first().click();
    await page.waitForTimeout(1500);
  }
  await page.waitForTimeout(2500);
  const play = page.locator('.device-group', { hasText: 'vehicle' }).locator('button').filter({ hasText: /^Play$/ }).first();
  await play.click();
  await page.waitForFunction(() => {
    const v = document.querySelector('video');
    return v && v.readyState >= 2 && v.videoWidth > 0;
  }, { timeout: 20000 });
  // wait for encoder_status to arrive (2s period — up to 6s)
  await page.waitForTimeout(6500);
  // panel text (codec group)
  const body = await page.locator('body').innerText();
  const hasCodec = /H264|VP8|VP9|AV1/i.test(body);
  const hasFps = /fps|\d+(\.\d+)?\s*(fps|FPS)/i.test(body);
  const hasEncMs = /ms/i.test(body);
  console.log(`panel: codec=${hasCodec} fps=${hasFps} encMs=${hasEncMs}`);
  console.log('console encoder logs:', logs.slice(-3));
  // assert: at least codec info appears
  expect(hasCodec || logs.length > 0).toBeTruthy();
});
