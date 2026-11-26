import { test, expect, type Page } from '@playwright/test';

// V43: mini-stats card = prototype .tile-stats
//   position: left 8 / top 30 (below the 28px bar -> never covers the title)
//   background rgba(8,12,20,.58), 1px rgba(255,255,255,.08), radius 4, padding 6/9/7
//   grid: auto 40px auto 40px, column-gap 8, mono 11px
//   keys rgba(255,255,255,.5), values rgba(255,255,255,.95) right-aligned
//   content: FPS RES BIT JIT LAT RTT DEC LOSS (DEC has no source -> em dash)
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);

test.describe('mini-stats card (prototype .tile-stats)', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.wall-grid .video-player.nvr').first()).toBeVisible();
    await page.locator('[aria-label="Stats"]').click(); // opt-in (V42)
    await expect(page.locator('.wall-grid.stats-on')).toBeVisible();
  });

  test('sits below the bar and never covers the title', async ({ page }) => {
    const bar = (await page.locator('.video-player.nvr .vp-top-bar').first().boundingBox())!;
    const card = (await page.locator('.video-player.nvr .vp-mini-stats').first().boundingBox())!;
    expect(card.y).toBeGreaterThanOrEqual(bar.y + bar.height - 1);
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'top')).toBe('30px');
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'left')).toBe('8px');
  });

  test('translucent scrim + 1px hairline, r4, 6/9/7 padding, mono 11px', async ({ page }) => {
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'background-color')).toBe('rgba(8, 12, 20, 0.58)');
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'border-radius')).toBe('4px');
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'padding')).toBe('6px 9px 7px');
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'font-size')).toBe('11px');
    const ff = await style(page, '.video-player.nvr .vp-mini-stats', 'font-family');
    expect(ff).toMatch(/mono|Mono|Consolas|Menlo/);
  });

  test('4-column grid of 8 pairs with the prototype labels and colours', async ({ page }) => {
    const cols = await style(page, '.video-player.nvr .ms-grid', 'grid-template-columns');
    expect(cols).toMatch(/40px .*40px/);
    expect(await style(page, '.video-player.nvr .ms-grid', 'column-gap')).toBe('8px');
    expect(await style(page, '.video-player.nvr .ms-k', 'color')).toBe('rgba(255, 255, 255, 0.5)');
    expect(await style(page, '.video-player.nvr .ms-v', 'color')).toBe('rgba(255, 255, 255, 0.95)');
    expect(await style(page, '.video-player.nvr .ms-v', 'text-align')).toBe('right');
    const first = page.locator('.video-player.nvr').first();
    const keys = await first.locator('.ms-grid .ms-k').allTextContents();
    expect(keys).toEqual(['FPS', 'RES', 'BIT', 'JIT', 'LAT', 'RTT', 'DEC', 'LOSS']);
    const dec = await first.locator('.ms-cell', { hasText: 'DEC' }).locator('.ms-v').textContent();
    expect(dec).toBe('—'); // no decoder-time source -> honest placeholder
    // the centered idle line is suppressed while the card is on (no duplicated info)
    expect(await style(page, '.video-player.nvr .tile-idle', 'opacity')).toBe('0');
  });
});
