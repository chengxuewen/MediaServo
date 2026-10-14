import { test, expect } from '@playwright/test';

test.describe('Admin Layout', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
  });

  test('renders header and sidebar navigation', async ({ page }) => {
    await expect(page.locator('header')).toContainText('MediaServo Admin');
    await expect(page.locator('nav a', { hasText: 'Dashboard' })).toBeVisible();
    await expect(page.locator('nav a', { hasText: 'Settings' })).toBeVisible();
  });

  test('dashboard is the default route', async ({ page }) => {
    await expect(page.locator('nav a.active')).toContainText('Dashboard');
  });

  test('navigates to settings via sidebar', async ({ page }) => {
    await page.locator('nav a', { hasText: 'Settings' }).click();
    await expect(page).toHaveURL('/settings');
    await expect(page.locator('nav a.active')).toContainText('Settings');
  });
});

test.describe('Dashboard Page', () => {
  test('shows dashboard content on load', async ({ page }) => {
    // storageState (global-setup login) provides the admin token — PIT-103: a signed-in state is still required after auth.
    await page.goto('/');
    await page.goto('/');
    // The dashboard shows "No active devices" when empty, or stats cards when live
    await expect(page.locator('.dashboard')).toBeVisible();
  });
});

test.describe('Settings Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/settings');
  });

  test('has token input and save button', async ({ page }) => {
    await expect(page.locator('h2')).toContainText('Settings');
    await expect(page.locator('input.token-input')).toBeVisible();
    await expect(page.locator('button', { hasText: 'Save Token' })).toBeVisible();
  });

  test('save button is clickable with empty input (no-op)', async ({ page }) => {
    const tokenSection = page.locator('.setting-group', { hasText: 'Admin Token' });
    await tokenSection.locator('button', { hasText: 'Save Token' }).click();
    // No token status appears in the token section because input was empty
    await expect(tokenSection.locator('.token-status')).toHaveCount(0);
  });

  test('saves and clears a token', async ({ page }) => {
    const tokenSection = page.locator('.setting-group', { hasText: 'Admin Token' });
    await tokenSection.locator('input.token-input').fill('test-jwt-token');
    await tokenSection.locator('button', { hasText: 'Save Token' }).click();
    await expect(tokenSection.locator('.token-status')).toContainText('Token saved');
    // Clear button appears after save
    const clearBtn = tokenSection.locator('button', { hasText: 'Clear' });
    await expect(clearBtn).toBeVisible();
    await clearBtn.click();
    await expect(tokenSection.locator('.token-status')).toHaveCount(0);
    await expect(tokenSection.locator('button', { hasText: 'Clear' })).toHaveCount(0);
  });
});
