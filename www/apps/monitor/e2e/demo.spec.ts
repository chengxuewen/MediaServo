import { test, expect } from '@playwright/test';

// V8: labeled simulated preview renders the prototype-like rich state (tiles, idle
// codec text, degraded border, LIVE badges, recording blocks) and is clearly labeled.
test('demo mode renders rich labeled preview', async ({ page }) => {
  await page.goto('/?demo=1');
  // V20: Demo/theme buttons removed from the wall toolbar (theme lives in admin Settings).
  await expect(page.locator('.demo-toggle')).toHaveCount(0);
  await expect(page.locator('.wall-grid .video-player')).toHaveCount(5);
  await expect(page.locator('.tile-idle').first()).toBeVisible();
  await expect(page.locator('.video-player.degraded')).toHaveCount(1);
  await expect(page.locator('.tile-live.live').first()).toBeVisible();
  await expect(page.locator('.wall-offline')).toHaveCount(1); // SIGNAL LOST
  // V24: the playback bar is off by default — open it from the stage-bar toggle.
  await page.locator('[aria-label="Toggle playback bar"]').click();
  await expect(page.locator('.tl-block').first()).toBeVisible();
  await expect(page.locator('.monitor-home h1')).toHaveText(/Monitor Center/);
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

// V17 geometry guard: in the 1+5 featured layout the "big" tile must be 2 small
// columns wide (plus one gap) and the small rows must share equal heights.
test('1+5 featured layout geometry (big spans 2 cols, rows even)', async ({ page }) => {
  await page.goto('/?demo=1');
  await page.locator('.layout-menu').click();
  await page.getByRole('option', { name: '1 + 5', exact: true }).click();
  await expect(page.locator('.wall-grid .wall-cell, .wall-grid .wall-offline').first()).toBeVisible();

  const geo = await page.evaluate(() => {
    const grid = document.querySelector('.wall-grid')!;
    const gap = parseFloat(getComputedStyle(grid).columnGap || '0');
    const cells = [...grid.querySelectorAll<HTMLElement>('.wall-cell, .wall-offline, .tile-empty')];
    const rects = cells.map((e) => { const r = e.getBoundingClientRect(); return { w: Math.round(r.width), h: Math.round(r.height), top: Math.round(r.top), left: Math.round(r.left), area: getComputedStyle(e).gridArea }; });
    const big = rects.find((r) => r.area.startsWith('big'))!;
    const small = rects.filter((r) => r.area !== big.area);
    const smallW = Math.max(...small.map((r) => r.w));
    const sameRow = small.filter((r) => Math.abs(r.top - small[0].top) < 2).map((r) => r.h);
    return { gap, bigW: big.w, smallW, bigH: big.h, smallH: small[0].h, rowHSpread: Math.max(...sameRow) - Math.min(...sameRow) };
  });
  // big spans two small columns + the gap between them
  expect(Math.abs(geo.bigW - (geo.smallW * 2 + geo.gap))).toBeLessThanOrEqual(3);
  // small cells on the same row share a height
  expect(geo.rowHSpread).toBeLessThanOrEqual(2);
  expect(geo.smallH).toBeGreaterThan(0);
});

// V18 guard: the monitor page must NOT scroll vertically — the wall scales to the
// viewport (prototype behaviour) instead of growing a document scrollbar.
test('monitor page has no vertical document scroll', async ({ page }) => {
  await page.goto('/?demo=1');
  await expect(page.locator('.wall-grid .video-player').first()).toBeVisible();
  const s = await page.evaluate(() => ({ sh: document.documentElement.scrollHeight, ih: window.innerHeight }));
  expect(s.sh).toBeLessThanOrEqual(s.ih + 1);
});