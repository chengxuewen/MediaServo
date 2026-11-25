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
    const logo = b('.header .logo'); const page2 = b('.header-page'); const head = b('.header');
    return {
      cyDelta: Math.abs((logo.top + logo.height / 2) - (page2.top + page2.height / 2)),
      ruleSpans: Math.round(page2.height) >= Math.round(head.height) - 1,
      border: getComputedStyle(document.querySelector('.header-page')!).borderLeftWidth,
    };
  });
  expect(geo.cyDelta).toBeLessThanOrEqual(2);
  expect(geo.ruleSpans).toBe(true);
  expect(geo.border).toBe('1px');
  // hierarchy: the page name must be visibly smaller than the brand
  const sizes = await page.evaluate(() => ({
    brand: parseFloat(getComputedStyle(document.querySelector('.header .logo')!).fontSize),
    page: parseFloat(getComputedStyle(document.querySelector('.header-page')!).fontSize),
  }));
  expect(sizes.page).toBeLessThanOrEqual(sizes.brand - 2);
});