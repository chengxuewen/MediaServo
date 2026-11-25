import { test, expect } from '@playwright/test';

// A (shell-stays): Monitor is an in-shell SPA route — clicking it must NOT full-page
// jump; the admin sidebar/header persist and the monitor face renders in the content.
// (admin project storageState pre-authenticates as admin.)
test('Monitor nav stays in the admin shell (SPA route)', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.sidebar')).toBeVisible();
  await page.locator('nav a', { hasText: 'Monitor' }).click();
  await expect(page).toHaveURL(/\/monitor/);
  await expect(page.locator('.sidebar')).toBeVisible(); // shell unchanged
  await expect(page.locator('.monitor-home')).toBeVisible();
  // back to Overview unmounts the face, shell still present
  await page.locator('nav a', { hasText: 'Overview' }).click();
  await expect(page.locator('.monitor-home')).toHaveCount(0);
  await expect(page.locator('.sidebar')).toBeVisible();
});