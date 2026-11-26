import { test, expect } from './host-fixture';
import { readyProducers } from './flow';

// B6 finishing: global stats toggle + single admin WS (M6). V9b (K12=A): the per-stream
// tree row (M5 bitrate) was retired with the rail device tree — not in the prototype.
test.describe('Monitor finishing (B6)', () => {
  test('global stats toggle hides/shows tile stats', async ({ page, hostProducer }) => {
    await page.goto('/');
    await readyProducers(page, hostProducer.room);
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .vp-mini-stats')).toHaveCount(1, { timeout: 15000 });

    await page.locator('.global-stats-toggle').click();
    await expect(page.locator('.wall-grid.stats-hidden')).toBeVisible();
    await expect(page.locator('.wall-grid .vp-mini-stats')).toBeHidden();
    await page.locator('.global-stats-toggle').click();
    await expect(page.locator('.wall-grid .vp-mini-stats')).toBeVisible();
  });

  test('single admin events WS connection (M6)', async ({ page }) => {
    // Count ACTIVE connections (open minus close): StrictMode dev double-mounts the
    // single Home useAdminWS hook (2 handshakes, first closed) — active must be 1.
    let active = 0;
    page.on('websocket', (ws) => {
      if (!ws.url().includes('/api/admin/events')) return;
      active++;
      ws.on('close', () => { active--; });
    });
    await page.goto('/');
    await expect(page.locator('.monitor-home')).toBeVisible();
    await page.waitForTimeout(2500);
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
    await readyProducers(page, hostProducer.room);
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
    await page.locator('.wall-grid .video-player').first().hover();
    await page.locator('.tile-actions [aria-label="Zoom this stream"]').click();
    await expect(page.locator('.wall-grid [aria-label="Exit zoom"]')).toBeVisible(); // V38
    await page.keyboard.press('Escape');
    await expect(page.locator('.wall-grid [aria-label="Exit zoom"]')).toHaveCount(0);
  });
});