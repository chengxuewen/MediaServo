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