import { test, expect, type Page } from '@playwright/test';

// V46: toolbar glyphs = prototype symbols (verbatim), sized like the prototype .ic.
//   #i-panel-left  rect(3,4,18,16 rx2) + M9 4v16
//   #i-panel-right rect(3,4,18,16 rx2) + M15 4v16
//   #i-timeline    rect(3,14,18,6 rx1) + M3 6h18 M7 14v-3 M12 14v-3 M17 14v-3
//   #i-refresh     M20 12a8 8 0 1 1-2.34-5.66 + M20 4v4h-4
//   #i-swap        M16 3l4 4-4 4M20 7H4M8 21l-4-4 4-4M4 17h16
//   #i-info        circle(12,12,9) + M12 11v6 M12 7.5v.5
//   .ic 15px, stroke 1.8, fill none; .pager .pg-label min-width 60 centred; gap 6
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);
const shape = (page: Page, sel: string) =>
  page.locator(sel).first().evaluate((el) => ({
    rects: [...el.querySelectorAll('rect')].map((r) => `${r.getAttribute('x')},${r.getAttribute('y')},${r.getAttribute('width')},${r.getAttribute('height')},${r.getAttribute('rx')}`),
    circles: [...el.querySelectorAll('circle')].map((c) => `${c.getAttribute('cx')},${c.getAttribute('cy')},${c.getAttribute('r')}`),
    paths: [...el.querySelectorAll('path')].map((p) => p.getAttribute('d')),
  }));

test.describe('toolbar glyphs vs prototype', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.wall-toolbar')).toBeVisible();
  });

  test('region toggles are #i-panel-left / #i-panel-right / #i-timeline', async ({ page }) => {
    const left = await shape(page, '[aria-label="Toggle stream rail"] svg');
    expect(left.rects[0]).toBe('3,4,18,16,2');
    expect(left.paths[0]).toBe('M9 4v16');

    const right = await shape(page, '[aria-label="Toggle alert panel"] svg');
    expect(right.rects[0]).toBe('3,4,18,16,2');
    expect(right.paths[0]).toBe('M15 4v16');

    const bottom = await shape(page, '[aria-label="Toggle playback bar"] svg');
    expect(bottom.rects[0]).toBe('3,14,18,6,1');
    expect(bottom.paths[0]).toBe('M3 6h18M7 14v-3M12 14v-3M17 14v-3');
  });

  test('carousel / sync / stats glyphs are #i-refresh / #i-swap / #i-info', async ({ page }) => {
    const car = await shape(page, '[aria-label="Carousel"] svg');
    expect(car.paths[0]).toBe('M20 12a8 8 0 1 1-2.34-5.66');
    expect(car.paths[1]).toBe('M20 4v4h-4');

    const sync = await shape(page, '[aria-label="Sync (deferred)"] svg');
    expect(sync.paths[0]).toBe('M16 3l4 4-4 4M20 7H4M8 21l-4-4 4-4M4 17h16');

    const info = await shape(page, '[aria-label="Stats"] svg');
    expect(info.circles[0]).toBe('12,12,9');
    expect(info.paths[0]).toBe('M12 11v6M12 7.5v.5');
  });

  test('.ic is 15px stroked 1.8; pager label 60px centred, gap 6', async ({ page }) => {
    const ic = (await page.locator('.wall-toolbar .ic').first().boundingBox())!;
    expect(Math.round(ic.width)).toBe(15);
    expect(Math.round(ic.height)).toBe(15);
    expect(await style(page, '.wall-toolbar .ic', 'stroke-width')).toBe('1.8px');
    expect(await style(page, '.wall-toolbar .ic', 'fill')).toBe('none');

    expect(await style(page, '.wt-page', 'min-width')).toBe('60px');
    expect(await style(page, '.wt-page', 'text-align')).toBe('center');
    expect(await style(page, '.wt-pager', 'gap')).toBe('6px');
  });
});
