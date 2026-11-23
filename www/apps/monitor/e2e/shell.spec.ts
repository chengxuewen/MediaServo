import { test, expect } from '@playwright/test';

// Authenticated shell smoke (monitor-admin project, admin storageState from
// globalSetup — no per-test login). Proves the thin shell renders for a signed-in
// admin: topbar brand + identity + honest home banner + logout affordance.
test.describe('Monitor Shell (authenticated)', () => {
  test('topbar brand + identity + honest home banner render', async ({ page }) => {
    await page.goto('/');

    await expect(page.locator('.header .logo')).toContainText('MediaServo Monitor');
    await expect(page.locator('.header .version')).toContainText('[admin]');
    await expect(page.locator('.monitor-home')).toBeVisible();
    await expect(page.locator('.logout-btn')).toBeVisible();
  });
});