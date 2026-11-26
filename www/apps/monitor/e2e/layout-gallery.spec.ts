import { test, expect, type Page } from '@playwright/test';

// V53: layout picker + wall templates mirror portal-unified.html.
//   gallery: 4 columns / 300px / pad 6 / gap 4 / 1px #B9C1CE / r8; option pad 6 r6;
//   selected = accent tint + accent ink + visible check; thumb 48x32 gap 1; label 10px mono;
//   order Auto | 1 4 9 16 25 36 | 1+3 1+5 1+7 1+9 1+12 | Custom (separators between groups).
//   wall: cols/areas per prototype (1+3: big 1x3 of 2 cols; 1+5: 3 cols; 1+7: 4; 1+9: 5; 1+12: 6).
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);
const ORDER = ['Auto', '1', '4', '9', '16', '25', '36', '1+3', '1+5', '1+7', '1+9', '1+12', 'Custom'];

async function openGallery(page: Page) {
  await page.locator('.layout-menu').click();
  await expect(page.locator('.layout-gallery')).toBeVisible();
}
async function pick(page: Page, id: string) {
  await openGallery(page);
  await page.locator('.layout-gallery [role="option"]', { hasText: new RegExp(`^${id.replace('+', '\\+')}$`) }).click();
  await expect(page.locator('.layout-gallery')).toHaveCount(0);
}

test.describe('layout gallery (prototype #colsPop)', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.wall-grid .video-player.nvr').first()).toBeVisible();
  });

  test('gallery is the prototype 4-column thumbnail grid in the prototype order', async ({ page }) => {
    await openGallery(page);
    const g = page.locator('.layout-gallery');
    expect(Math.round((await g.boundingBox())!.width)).toBe(300);
    expect((await style(page, '.layout-gallery', 'grid-template-columns')).split(' ').length).toBe(4);
    expect(await style(page, '.layout-gallery', 'padding')).toBe('6px');
    expect(await style(page, '.layout-gallery', 'gap')).toBe('4px');
    expect(await style(page, '.layout-gallery', 'border-radius')).toBe('8px');
    expect(await style(page, '.layout-gallery', 'border-top-color')).toBe('rgb(185, 193, 206)');
    const opts = page.locator('.layout-gallery [role="option"]');
    await expect(opts).toHaveCount(13);
    expect(await opts.evaluateAll((els) => els.map((e) => e.querySelector('.lo-label')!.textContent!.trim()))).toEqual(['Auto', ...ORDER.slice(1)]);
    await expect(page.locator('.layout-gallery .lo-sep')).toHaveCount(3);
    // option + thumb + label metrics
    const o = (await opts.first().boundingBox())!;
    expect(o.width).toBeGreaterThanOrEqual(60); expect(o.width).toBeLessThanOrEqual(72);
    expect(await style(page, '.layout-gallery .lo-opt', 'padding')).toBe('6px');
    expect(await style(page, '.layout-gallery .lo-opt', 'border-radius')).toBe('6px');
    const t = (await page.locator('.layout-gallery .lo-thumb').first().boundingBox())!;
    expect(Math.round(t.width)).toBe(48); expect(Math.round(t.height)).toBe(32);
    expect(await style(page, '.layout-gallery .lo-label', 'font-size')).toBe('10px');
  });

  test('selected option = accent tint + accent ink + visible check; others have no check', async ({ page }) => {
    await pick(page, '9'); // a thumbnail layout (Auto has no .lo-thumb)
    await openGallery(page);
    const sel = page.locator('.layout-gallery [role="option"][aria-selected="true"]');
    await expect(sel).toHaveCount(1);
    expect(await sel.evaluate((e) => getComputedStyle(e).backgroundColor)).toBe('rgb(238, 241, 255)');
    expect(await sel.evaluate((e) => getComputedStyle(e).color)).toBe('rgb(67, 97, 238)');
    expect(await sel.locator('.lo-check').evaluate((e) => getComputedStyle(e).opacity)).toBe('1');
    expect(await sel.locator('.lo-thumb i').first().evaluate((e) => getComputedStyle(e).backgroundColor)).toBe('rgb(67, 97, 238)');
    const other = page.locator('.layout-gallery [role="option"][aria-selected="false"]').first();
    expect(await other.locator('.lo-check').evaluate((e) => getComputedStyle(e).opacity)).toBe('0');
    // the Auto thumb is the stroked #i-grid glyph (4 rects, 20px)
    const auto = page.locator('.layout-gallery [role="option"]', { hasText: 'Auto' }).locator('.lo-auto');
    expect(await auto.locator('rect').count()).toBe(4);
    expect(Math.round((await auto.boundingBox())!.width)).toBe(20);
  });

  for (const [id, cols, bigC, bigR] of [['1+3', 2, 1, 3], ['1+5', 3, 2, 2], ['1+7', 4, 2, 2], ['1+9', 5, 2, 3], ['1+12', 6, 2, 3]] as const) {
    test(`wall template ${id}: ${cols} columns, big cell spans ${bigC}x${bigR}`, async ({ page }) => {
      await pick(page, id);
      const grid = page.locator('.wall-grid');
      expect((await style(page, '.wall-grid', 'grid-template-columns')).split(' ').length).toBe(cols);
      const cells = grid.locator('.wall-cell, .wall-offline, .tile-empty');
      const big = (await cells.nth(1).boundingBox())!; // index 0 is the big slot? cells follow playing order: first = big
      const bigBox = (await cells.nth(0).boundingBox())!;
      const smallBox = (await cells.nth(1).boundingBox())!;
      const gap = parseFloat(await style(page, '.wall-grid', 'column-gap'));
      void big;
      expect(Math.abs(bigBox.width - (smallBox.width * bigC + gap * (bigC - 1)))).toBeLessThanOrEqual(3);
      expect(Math.abs(bigBox.height - (smallBox.height * bigR + gap * (bigR - 1)))).toBeLessThanOrEqual(3);
    });
  }

  test('sym templates 16 / 25 / 36 render their column counts', async ({ page }) => {
    for (const [id, cols] of [['16', 4], ['25', 5], ['36', 6]] as const) {
      await pick(page, id);
      expect((await style(page, '.wall-grid', 'grid-template-columns')).split(' ').length).toBe(cols);
    }
  });
});
