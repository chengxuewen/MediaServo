import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import * as fs from 'fs';

// H1/H6 refresh-free self-heal regression (PIT-161/162 codified as a Hyrum guard).
// Prereq: deployed instance out/host (msrtc-host) + native server 9800 + host already streaming.
// Regular CI has no deployed instance → set env RUN_HOST_HEAL=1 to opt in.
const HOST_DIR = process.env.HOST_DIR ?? '/home/maxsense/Documents/ms_rtc/out/host';
test.describe('restart heal (no refresh)', () => {
  test.skip(process.env.RUN_HOST_HEAL !== '1', 'requires a deployed out/host instance (set RUN_HOST_HEAL=1)');
  test.setTimeout(180_000);
  const frames = (page) => page.evaluate(() => [...document.querySelectorAll('video')].map(v => ({
    rs: v.readyState, w: v.videoWidth, fr: v.getVideoPlaybackQuality().totalVideoFrames,
  })));
  test('host restart → picture recovers automatically without refresh', async ({ page }) => {
    await page.goto('/');
    await page.locator('input.login-input[placeholder="username"]').fill('admin');
    await page.locator('input.login-input[placeholder="password"]').fill('admin123');
    await page.locator('button', { hasText: 'Login' }).click();
    await page.waitForSelector('.dashboard');
    await page.locator('button.btn-play', { hasText: 'Play All' }).first().click();
    await page.waitForSelector('video', { timeout: 15000 });
    await page.waitForTimeout(6000);
    execSync('./msrtc-host restart', { cwd: HOST_DIR, timeout: 60000 });
    let prev = null;
    await expect
      .poll(
        async () => {
          const cur = await frames(page);
          const ok = !!prev && cur.length >= 2 && cur.every((v, i) => v.rs >= 2 && v.w > 0 && v.fr > prev[i].fr + 30);
          prev = cur;
          return ok;
        },
        { timeout: 90000, intervals: [4000] },
      )
      .toBe(true);
    if (!fs.existsSync(HOST_DIR)) test.skip();
  });
});
