import { test, expect } from './host-fixture';

// B6 (monitor-prototype-parity) finishing: global stats toggle, per-stream tree
// metrics (M5/O4), and the single admin WS consolidation (M6 — was 3 connections).
test.describe('Monitor finishing (B6)', () => {
  test('global stats toggle hides/shows tile stats', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .vp-mini-stats')).toHaveCount(1, { timeout: 15000 });

    await page.locator('.global-stats-toggle').click();
    await expect(page.locator('.wall-grid.stats-hidden')).toBeVisible();
    await expect(page.locator('.wall-grid .vp-mini-stats')).toBeHidden();
    await page.locator('.global-stats-toggle').click();
    await expect(page.locator('.wall-grid .vp-mini-stats')).toBeVisible();
  });

  test('per-stream bitrate renders on the tree row (M5)', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });
    await expect(page.locator('.st-bitrate').first()).toBeVisible();
  });

  test('single admin events WS connection (M6)', async ({ page }) => {
    // Count ACTIVE connections (open minus close): StrictMode dev double-mounts the
    // single Home useAdminWS hook (2 handshakes, first closed) — active must be 1.
    // Before M6 this was 3 hooks (Home/Timeline/AlertPanel) → 3 active.
    let active = 0;
    page.on('websocket', (ws) => {
      if (!ws.url().includes('/api/admin/events')) return;
      active++;
      ws.on('close', () => { active--; });
    });
    await page.goto('/');
    await expect(page.locator('.monitor-home')).toBeVisible();
    await page.waitForTimeout(2500); // let StrictMode remount + handshake settle
    expect(active).toBe(1);
  });

  test('V5 theme toggle switches to dark (K8)', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.monitor-home')).toBeVisible();
    const bgLight = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    await page.locator('.theme-toggle').click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    const bgDark = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    expect(bgDark).not.toBe(bgLight);
  });

  test('V5 Esc exits single-tile zoom (I8)', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
    await page.locator('.wall-grid .video-player').first().hover();
    await page.locator('.tile-actions [aria-label="Zoom this stream"]').click();
    await expect(page.locator('.wt-exit-zoom')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.wt-exit-zoom')).toHaveCount(0);
  });
});