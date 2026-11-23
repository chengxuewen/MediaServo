import { test, expect } from '@playwright/test';

test.describe('Admin Layout', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
  });

  test('renders header and sidebar navigation', async ({ page }) => {
    await expect(page.locator('header')).toContainText('MediaServo');
    await expect(page.locator('nav a', { hasText: 'Overview' })).toBeVisible();
    await expect(page.locator('nav a', { hasText: 'Admin' })).toBeVisible();
  });

  test('dashboard is the default route', async ({ page }) => {
    await expect(page.locator('nav a.active')).toContainText('Overview');
  });

  test('navigates to settings via the Admin hub (D12=A)', async ({ page }) => {
    await page.locator('nav a', { hasText: 'Admin' }).click();
    await expect(page).toHaveURL('/admin');
    await page.getByRole('tab', { name: 'Settings' }).click();
    await expect(page.getByRole('tab', { name: 'Settings' })).toHaveAttribute('aria-selected', 'true');
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

// W1 (admin-prototype-restyle): overview = prototype health summary, not a
// resource counter; the device long-list lives in Admin → Devices.
test.describe('Overview health summary (W1 prototype parity)', () => {
  test('six health stats replace the resource counters', async ({ page }) => {
    await page.goto('/');
    const labels = page.locator('.ov-stat .ov-stat-k');
    await expect(labels).toHaveCount(6);
    await expect(labels).toHaveText([
      'Devices online', 'Degraded', 'Offline', 'Pending approval', 'Live rooms', 'Active alerts',
    ]);
    // old resource-counter face is gone
    await expect(page.locator('.stats-card')).toHaveCount(0);
  });

  test('fleet / alerts / quick-check cards render; no device table on overview', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.ov-fleet .ov-card-title')).toHaveText('Fleet');
    await expect(page.locator('.ov-fleet-summary')).toContainText('vehicles');
    await expect(page.getByRole('button', { name: 'Open fleet lens' })).toBeVisible();
    await expect(page.locator('.ov-card-title', { hasText: 'Alerts · last 24 h' })).toBeVisible();
    await expect(page.locator('.ov-card-title', { hasText: 'Quick check' })).toBeVisible();
    await expect(page.locator('.ov-chip')).toHaveText('≤4 tiles');
    // prototype overview does not list devices
    await expect(page.getByRole('heading', { name: 'Active Devices' })).toHaveCount(0);
    await expect(page.locator('.device-list')).toHaveCount(0);
  });

  test('device wall relocated to the Admin hub Devices tab; fleet lens deep-links', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Open fleet lens' }).click();
    await expect(page).toHaveURL('/admin');
    await expect(page.getByRole('tab', { name: 'Fleet' })).toHaveAttribute('aria-selected', 'true');
    await page.getByRole('tab', { name: 'Devices' }).click();
    await expect(page.getByRole('heading', { name: 'Active Devices' })).toBeVisible();
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
