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