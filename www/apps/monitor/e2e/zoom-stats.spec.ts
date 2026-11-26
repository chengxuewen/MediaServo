import { test, expect, type Page } from '@playwright/test';

// V38: two prototype behaviours that were broken:
//  1) A ZOOMED tile shows ONE button — "Exit zoom" (#i-zoom-out) — always visible, so
//     the user can shrink back (we showed 2x2 + remove and had no way out).
//  2) The toolbar stats toggle PINS the mini-stats card on (prototype .tiles.show-stats);
//     default stays hover-revealed.
const ICON_ZOOM_OUT = 'M9 3v6H3M15 3v6h6M21 15h-6v6M3 15h6v6';
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);

test.describe('zoom + stats toggle', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.wall-grid .video-player.nvr').first()).toBeVisible();
  });

  test('zoomed tile shows only "Exit zoom" (always visible) and clicking it restores the grid', async ({ page }) => {
    const tile = page.locator('.wall-grid .video-player.nvr').first();
    await tile.hover();
    await tile.getByRole('button', { name: 'Zoom this stream' }).click();

    const zoomed = page.locator('.wall-grid .video-player.nvr.zoomed').first();
    await expect(zoomed).toBeVisible();
    // V40: prototype .tile.zoomed — ONE button (Exit zoom), no remove, no status glyph
    await expect(zoomed.locator('.tile-actions .tile-btn')).toHaveCount(1);
    await expect(zoomed.getByRole('button', { name: 'Exit zoom' })).toBeVisible();
    await expect(zoomed.getByRole('button', { name: 'Remove from wall' })).toHaveCount(0);
    await expect(zoomed.locator('[aria-label="Enlarge to 2×2"]')).toHaveCount(0);
    await expect(zoomed.locator('.tile-glyph')).toHaveCount(0); // no green dot when zoomed
    await expect(zoomed.locator('.tile-drift')).toHaveText(''); // no fake drift
    // prototype zoomed metrics: 28x28 button, 15px glyph, bar padding-right 40
    const zb = await zoomed.getByRole('button', { name: 'Exit zoom' }).boundingBox();
    expect(Math.round(zb!.width)).toBe(28);
    expect(Math.round(zb!.height)).toBe(28);
    const ic = await zoomed.locator('[aria-label="Exit zoom"] .ic').boundingBox();
    expect(Math.round(ic!.width)).toBe(15);
    expect(await style(page, '.video-player.nvr.zoomed .vp-top-bar', 'padding')).toBe('6px 40px 6px 8px');
    expect(await style(page, '.video-player.nvr.zoomed .tile-actions', 'opacity')).toBe('1');
    const d = await zoomed.locator('[aria-label="Exit zoom"] path').first().getAttribute('d');
    expect(d).toBe(ICON_ZOOM_OUT);

    await zoomed.getByRole('button', { name: 'Exit zoom' }).click();
    await expect(page.locator('.wall-grid .video-player.nvr.zoomed')).toHaveCount(0);
    expect(await page.locator('.wall-grid .video-player.nvr').count()).toBeGreaterThan(1);
  });

  test('stats toggle pins the mini-stats card on; default is hover-only', async ({ page }) => {
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'opacity')).toBe('0');
    await page.locator('[aria-label="Stats"]').click();
    await expect(page.locator('.wall-grid.stats-on')).toBeVisible();
    await page.mouse.move(5, 5); // not hovering any tile
    await expect.poll(async () => style(page, '.video-player.nvr .vp-mini-stats', 'opacity')).toBe('1');
    await page.locator('[aria-label="Stats"]').click();
    await expect(page.locator('.wall-grid.stats-on')).toHaveCount(0);
    await expect.poll(async () => style(page, '.video-player.nvr .vp-mini-stats', 'opacity')).toBe('0');
  });
});
