import { test, expect } from '@playwright/test';

// V21 (prototype nav): the sidebar collapses to a 58px icon rail via the header
// button, persists the choice, and auto-rails at <=900px (prototype media rule).
const width = async (page: import('@playwright/test').Page) =>
  (await page.locator('.sidebar').boundingBox())!.width;

test('nav collapses to 58px rail, persists, and auto-rails at <=900px', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.sidebar')).toBeVisible();
  await expect(page.locator('.nav-label').first()).toBeVisible();
  expect(await width(page)).toBeGreaterThan(120);

  // collapse -> icon rail, labels hidden
  await page.locator('.nav-collapse').click();
  await expect(page.locator('.nav-label').first()).toBeHidden();
  await expect.poll(async () => Math.round(await width(page))).toBeLessThanOrEqual(60);

  // persisted across reload
  await page.reload();
  await expect.poll(async () => Math.round(await width(page))).toBeLessThanOrEqual(60);

  // expand again
  await page.locator('.nav-collapse').click();
  await expect.poll(async () => Math.round(await width(page))).toBeGreaterThan(120);
  await expect(page.locator('.nav-label').first()).toBeVisible();

  // narrow viewport auto-rails even when the stored state is expanded
  await page.setViewportSize({ width: 820, height: 900 });
  await expect.poll(async () => Math.round(await width(page))).toBeLessThanOrEqual(60);
  await expect(page.locator('.nav-label').first()).toBeHidden();

  // back to wide -> restored expanded
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect.poll(async () => Math.round(await width(page))).toBeGreaterThan(120);
});

// V22 (prototype topbar): the brand is followed by the current page name.
test('header shows the current page name beside the brand', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.header-page')).toHaveText('Overview');
  await page.locator('nav a', { hasText: 'Monitor' }).click();
  await expect(page.locator('.header-page')).toHaveText('Monitor Center');
  await expect(page.locator('.sidebar')).toBeVisible();

  // V22c: vertical rule spans the bar and the label is vertically centred with the brand.
  const geo = await page.evaluate(() => {
    const b = (s: string) => document.querySelector(s)!.getBoundingClientRect();
    const rule = b('.header-rule'); const head = b('.header');
    const logos = b('.header .logo'); // icon+text flex box = the visual brand block
    const pageBox = b('.header-page');
    return {
      ruleSpans: Math.abs(Math.round(rule.height) - Math.round(head.height)) <= 1,
      ruleWidth: getComputedStyle(document.querySelector('.header-rule')!).width,
      align: getComputedStyle(document.querySelector('.header-left')!).alignItems,
      // V22f: both texts must be vertically CENTRED in the bar (not riding high).
      brandToCentre: Math.abs((logos.top + logos.height / 2) - (head.top + head.height / 2)),
      pageToCentre: Math.abs((pageBox.top + pageBox.height / 2) - (head.top + head.height / 2)),
    };
  });
  expect(geo.ruleSpans).toBe(true);
  expect(geo.ruleWidth).toBe('1px');
  expect(geo.align).toBe('center');

  // V22h: PIXEL truth check — screenshot the header and compare the ink bottoms of the
// brand vs the page label. Canvas font metrics mis-modelled the ascent (false 6px), so
// we measure the rendered pixels: both text runs must sit on the same baseline (±1px).
  const shot = await page.locator('.header').screenshot();
  const { PNG } = await import('pngjs');
  const png = PNG.sync.read(Buffer.from(shot));
  const lum = (x: number, y: number) => {
    const i = (y * png.width + x) * 4;
    return 0.2126 * png.data[i] + 0.7152 * png.data[i + 1] + 0.0722 * png.data[i + 2];
  };
  const inkBottom = (x0: number, x1: number) => {
    let bottom = -1;
    for (let y = 0; y < png.height; y++) for (let x = x0; x < x1; x++) if (lum(x, y) > 150) { bottom = y; break; }
    return bottom;
  };
  // brand word ≈ x 60..165, page label ≈ x 178..300 (css px; dpr-1 headless)
  const brandBottom = inkBottom(60, 165);
  const pageBottom = inkBottom(178, 300);
  expect(brandBottom).toBeGreaterThan(0);
  expect(Math.abs(brandBottom - pageBottom)).toBeLessThanOrEqual(1);
  // hierarchy: the page name must be visibly smaller than the brand
  const sizes = await page.evaluate(() => ({
    brand: parseFloat(getComputedStyle(document.querySelector('.header .logo')!).fontSize),
    page: parseFloat(getComputedStyle(document.querySelector('.header-page')!).fontSize),
  }));
  expect(sizes.page).toBeLessThanOrEqual(sizes.brand - 2);
});