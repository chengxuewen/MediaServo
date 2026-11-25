import { test, expect, type Page } from '@playwright/test';

// V29: responsive matrix — widths around every breakpoint, docking vs overlay,
// anchoring coordinates, mutual exclusion, and "no horizontal scroll".
// Everything is asserted from the DOM (position/rects), not screenshots, so it is
// deterministic in CI.
const WIDTHS = [1600, 1280, 1100, 1024, 1023, 900, 768, 767, 700, 640, 480];
const DOCK_MIN = 768; // >= this: three docked columns (prototype); below: overlay drawers

const rect = async (page: Page, sel: string) => {
  const b = await page.locator(sel).boundingBox();
  expect(b, `${sel} must exist`).toBeTruthy();
  return b!;
};
const css = (page: Page, sel: string, prop: string) =>
  page.locator(sel).evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);

test.describe('responsive matrix', () => {
  for (const w of WIDTHS) {
    test(`${w}px: docking mode, anchoring, no h-scroll`, async ({ page }) => {
      await page.setViewportSize({ width: w, height: 860 });
      // fresh region memory so the default state is exercised at every width
      await page.addInitScript(() => { try { localStorage.removeItem('mediaservo_monitor_regions'); } catch { /* ignore */ } });
      await page.goto('/?demo=1');
      await expect(page.locator('.wall-toolbar')).toBeVisible();

      // 1) never a horizontal document scrollbar
      const overflowX = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      expect(overflowX, 'no horizontal scroll').toBeLessThanOrEqual(1);

      // 2) wall always visible and reasonably sized
      const grid = await rect(page, '.wall-grid');
      expect(grid.width).toBeGreaterThan(150);

      const panel = await rect(page, '.monitor-three-panel');
      const bar = await rect(page, '.wall-toolbar');

      if (w >= DOCK_MIN) {
        // docked: rails are in-flow, share the toolbar's top, wall sits between them
        expect(await css(page, '.stream-rail', 'position')).toBe('static');
        expect(await css(page, '.mon-alerts', 'position')).toBe('static');
        const rail = await rect(page, '.stream-rail');
        const alerts = await rect(page, '.mon-alerts');
        expect(Math.abs(rail.y - bar.y), 'rail top == toolbar top').toBeLessThanOrEqual(2);
        expect(Math.abs(alerts.y - bar.y), 'alerts top == toolbar top').toBeLessThanOrEqual(2);
        expect(rail.x).toBeLessThan(grid.x);
        expect(grid.x + grid.width).toBeLessThanOrEqual(alerts.x + 1);
        // both rails reach the panel bottom (full height columns)
        expect(Math.abs(rail.y + rail.height - (panel.y + panel.height))).toBeLessThanOrEqual(2);
        expect(Math.abs(alerts.y + alerts.height - (panel.y + panel.height))).toBeLessThanOrEqual(2);

        // V30: a CLOSED rail must not shift the grid — canvas keeps column 2 and the
        // alerts stay on the RIGHT (this regressed at 768..1023 when only the left rail
        // was closed: canvas fell into the auto track at 0 width, alerts jumped left).
        await page.locator('[aria-label="Toggle stream rail"]').click(); // close left
        await expect(page.locator('.stream-rail')).toBeHidden();
        let g2 = await rect(page, '.wall-grid');
        let a2 = await rect(page, '.mon-alerts');
        expect(g2.width, 'grid keeps width when the left rail is closed').toBeGreaterThan(200);
        expect(a2.x, 'alerts stay right of the grid').toBeGreaterThanOrEqual(g2.x + g2.width - 1);
        expect(Math.abs(a2.x + a2.width - (panel.x + panel.width))).toBeLessThanOrEqual(2);

        await page.locator('[aria-label="Toggle stream rail"]').click();  // reopen left
        await page.locator('[aria-label="Toggle alert panel"]').click();  // close right
        await expect(page.locator('.mon-alerts')).toBeHidden();
        const r2 = await rect(page, '.stream-rail');
        g2 = await rect(page, '.wall-grid');
        expect(g2.width, 'grid keeps width when the right rail is closed').toBeGreaterThan(200);
        expect(g2.x, 'grid right of the rail').toBeGreaterThanOrEqual(r2.x + r2.width - 1);

        await page.locator('[aria-label="Toggle stream rail"]').click();  // both closed
        await expect(page.locator('.stream-rail')).toBeHidden();
        const g3 = await rect(page, '.wall-grid');
        expect(g3.width, 'grid takes the full panel width when both rails are closed').toBeGreaterThan(panel.width - 40);
        const ox = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        expect(ox).toBeLessThanOrEqual(1);
      } else {
        // overlay: closed by default; rails hidden
        await expect(page.locator('.stream-rail')).toBeHidden();
        await expect(page.locator('.mon-alerts')).toBeHidden();

        // open LEFT -> anchored inside the panel (left edge, below the toolbar)
        await page.locator('[aria-label="Toggle stream rail"]').click();
        await expect(page.locator('.stream-rail')).toBeVisible();
        expect(await css(page, '.stream-rail', 'position')).toBe('absolute');
        const rail = await rect(page, '.stream-rail');
        expect(Math.abs(rail.x - panel.x), 'drawer left edge = panel left').toBeLessThanOrEqual(1);
        expect(rail.y, 'drawer starts below the stage-bar').toBeGreaterThanOrEqual(bar.y + bar.height - 1);
        expect(rail.y).toBeLessThanOrEqual(bar.y + bar.height + 12);
        expect(Math.abs(rail.y + rail.height - (panel.y + panel.height)), 'drawer reaches bottom').toBeLessThanOrEqual(2);

        // open RIGHT -> left closes (mutual exclusion); right anchored to panel right
        await page.locator('[aria-label="Toggle alert panel"]').click();
        await expect(page.locator('.mon-alerts')).toBeVisible();
        await expect(page.locator('.stream-rail')).toBeHidden();
        const alerts = await rect(page, '.mon-alerts');
        expect(Math.abs(alerts.x + alerts.width - (panel.x + panel.width)), 'drawer right edge = panel right').toBeLessThanOrEqual(1);
        expect(alerts.y).toBeGreaterThanOrEqual(bar.y + bar.height - 1);

        // bottom playback opens below and the wall keeps a usable height
        await page.locator('[aria-label="Toggle playback bar"]').click();
        await expect(page.locator('.ctl-bar')).toBeVisible();
        const g2 = await rect(page, '.wall-grid');
        expect(g2.height).toBeGreaterThan(100);
      }
    });
  }
});