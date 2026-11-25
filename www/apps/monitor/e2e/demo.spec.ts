import { test, expect } from '@playwright/test';

// V8: labeled simulated preview renders the prototype-like rich state (tiles, idle
// codec text, degraded border, LIVE badges, recording blocks) and is clearly labeled.
test('demo mode renders rich labeled preview', async ({ page }) => {
  await page.goto('/?demo=1');
  await expect(page.locator('.demo-note')).toBeVisible();
  await expect(page.locator('.demo-note')).toContainText('NOT real streams');
  await expect(page.locator('.wall-grid .video-player')).toHaveCount(5);
  await expect(page.locator('.tile-idle').first()).toBeVisible();
  await expect(page.locator('.video-player.degraded')).toHaveCount(1);
  await expect(page.locator('.tile-live.live').first()).toBeVisible();
  await expect(page.locator('.wall-offline')).toHaveCount(1); // SIGNAL LOST
  await expect(page.locator('.tl-block').first()).toBeVisible();
  await expect(page.locator('.mh-bar h1')).toHaveText('Monitor Center');
});

// V16 geometry guard: live tiles, offline tiles and empty slots must share the SAME
// height (rows were uneven because a live tile = top bar + 16:9 body vs pure 16:9 slot).
test('wall cells are uniform height (live vs empty slot)', async ({ page }) => {
  await page.goto('/?demo=1');
  await expect(page.locator('.wall-grid .video-player').first()).toBeVisible();
  // Uniform layout (9) so every cell has the same column width — the 1+5 "big" tile is
  // intentionally larger. Live tiles and empty slots must still match (was +top-bar tall).
  await page.locator('.layout-menu').click();
  await page.getByRole('option', { name: '9', exact: true }).click();
  await expect(page.locator('.wall-grid .tile-empty').first()).toBeVisible();
  const heights = await page.evaluate(() => [...document.querySelectorAll('.wall-grid .wall-cell, .wall-grid .wall-offline, .wall-grid .tile-empty')]
    .map((e) => Math.round(e.getBoundingClientRect().height)));
  expect(heights.length).toBeGreaterThan(2);
  expect(Math.max(...heights) - Math.min(...heights)).toBeLessThanOrEqual(2);
});