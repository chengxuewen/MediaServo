import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import * as fs from 'fs';

// H1/H6 免刷新自愈回归（PIT-161/162 固化为 Hyrum 防线）。
// 前置: 部署实例 out/host（msrtc-host）+ native server 9800 + host 已在推流。
// 常规 CI 无部署实例 → 环境变量 RUN_HOST_HEAL=1 显式开启。
const HOST_DIR = process.env.HOST_DIR ?? '/home/maxsense/Documents/ms_rtc/out/host';
test.describe('restart heal (no refresh)', () => {
  test.skip(process.env.RUN_HOST_HEAL !== '1', '需要 out/host 部署实例（RUN_HOST_HEAL=1 开启）');
  test.setTimeout(180_000);
  const frames = (page) => page.evaluate(() => [...document.querySelectorAll('video')].map(v => ({
    rs: v.readyState, w: v.videoWidth, fr: v.getVideoPlaybackQuality().totalVideoFrames,
  })));
  test('host restart → 画面自动恢复，无需刷新', async ({ page }) => {
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
