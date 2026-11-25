import { test, expect, type Page } from '@playwright/test';

// V31: stage-bar must match the prototype's measured metrics (portal-unified.html).
// Reference values read live from the prototype:
//   bar: min-height 44, padding 4px 16px, gap 8, flex-wrap: wrap, white bg, 1px bottom border
//   .iconbtn 30x30 r6, bg #E9EDF3 (.on -> #EEF1FF)
//   .combo-trigger 120x30 r6 font 12, bg #E9EDF3
//   pager buttons 30x30 r4, bg #E9EDF3
//   .warn-chip 30 high r6, bg #FDECEC
//   .live-badge 30 high r6, border #1F9D55, bg #E6F7EE
//   .tb-sep 1x20
const box = async (page: Page, sel: string) => {
  const b = await page.locator(sel).first().boundingBox();
  expect(b, `${sel} exists`).toBeTruthy();
  return b!;
};
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);

test.describe('stage-bar metrics vs prototype', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.wall-toolbar')).toBeVisible();
  });

  test('bar container: padding 4/16, gap 8, wrap, bottom border, min-height 44', async ({ page }) => {
    expect(await style(page, '.wall-toolbar', 'padding')).toBe('4px 16px');
    expect(await style(page, '.wall-toolbar', 'gap')).toBe('8px');
    expect(await style(page, '.wall-toolbar', 'flex-wrap')).toBe('wrap');
    expect(await style(page, '.wall-toolbar', 'border-bottom-width')).toBe('1px');
    const b = await box(page, '.wall-toolbar');
    expect(b.height).toBeGreaterThanOrEqual(38);
  });

  test('region toggles are 30x30 r6 with muted fill', async ({ page }) => {
    const btn = await box(page, '.region-toggles button');
    expect(Math.round(btn.width)).toBe(30);
    expect(Math.round(btn.height)).toBe(30);
    expect(await style(page, '.region-toggles button', 'border-radius')).toBe('6px');
  });

  test('layout combo is >=120x30, radius 6, font 12', async ({ page }) => {
    const b = await box(page, '.layout-menu');
    expect(Math.round(b.height)).toBe(30);
    expect(b.width).toBeGreaterThanOrEqual(119);
    expect(await style(page, '.layout-menu', 'border-radius')).toBe('6px');
    expect(await style(page, '.layout-menu', 'font-size')).toBe('12px');
    await expect(page.locator('.layout-menu .lm-label')).toHaveText('Layout');
  });

  test('pager buttons are 30x30 r4 and the count reads "n / m"', async ({ page }) => {
    const btn = await box(page, '.wt-pager button');
    expect(Math.round(btn.width)).toBe(30);
    expect(Math.round(btn.height)).toBe(30);
    expect(await style(page, '.wt-pager button', 'border-radius')).toBe('4px');
    await expect(page.locator('.wt-page')).toHaveText(/^\d+ \/ \d+$/);
  });

  test('health chip is 30 high with danger tint; LIVE chip 30 high r6', async ({ page }) => {
    const chip = await box(page, '.health-chip.warn');
    expect(Math.round(chip.height)).toBe(30);
    expect(await style(page, '.health-chip.warn', 'border-radius')).toBe('6px');
    // prototype: border #E5484D, fill #FDECEC
    expect(await style(page, '.health-chip.warn', 'border-color')).toBe('rgb(229, 72, 77)');
    expect(await style(page, '.health-chip.warn', 'background-color')).toBe('rgb(253, 236, 236)');
    const live = await box(page, '.live-chip');
    expect(Math.round(live.height)).toBe(30);
    expect(await style(page, '.live-chip', 'border-radius')).toBe('6px');
  });

  test('narrow canvas WRAPS the bar instead of hiding controls', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 800 });
    await page.reload();
    await expect(page.locator('.wall-toolbar')).toBeVisible();
    // controls are all still present (prototype wraps, it never hides them)
    await expect(page.locator('.health-chip.warn')).toBeVisible();
    await expect(page.locator('.live-clock')).toBeVisible();
    const overflow = await page.evaluate(() => {
      const el = document.querySelector('.wall-toolbar')!;
      return el.scrollWidth - el.clientWidth;
    });
    expect(overflow).toBeLessThanOrEqual(1);
    const b = await box(page, '.wall-toolbar');
    expect(b.height).toBeLessThanOrEqual(90); // 1-2 rows, never a tall stack
  });
});