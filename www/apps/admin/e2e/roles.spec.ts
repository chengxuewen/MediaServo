import { test, expect } from '@playwright/test';

// H3: admin role view (storageState signed in as admin — global-setup).
// Prereq: Docker server (9800) + vite dev (5173).

test.describe('H3 admin role-aware views', () => {
  test('admin sees audio + vehicles nav, header shows role', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.header .version')).toContainText('admin [admin]');
    // W8 retired: Audio merged into Intercom; Rooms/Evidence are Intercom tabs
    await page.locator('nav a', { hasText: 'Intercom' }).click();
    await expect(page.getByRole('tab', { name: 'Rooms' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Evidence' })).toBeVisible();
    // D12=A: Fleet moved into the Admin hub (one Administration entry)
    await page.locator('nav a', { hasText: 'Admin' }).click();
    await expect(page.getByRole('tab', { name: 'Fleet' })).toBeVisible();
  });

  test('admin token keeps full access (write passes auth gate)', async ({ page }) => {
    await page.goto('/');
    const token = await page.evaluate(() => localStorage.getItem('mediaservo_admin_token'));
    const deleteStatus = await page.evaluate(async (tok) => {
      const res = await fetch('/api/admin/rooms/nope', { method: 'DELETE', headers: { Authorization: `Bearer ${tok}` } });
      return res.status;
    }, token);
    expect(deleteStatus).toBe(404); // read-only block: room not found → falls through to room lookup (auth passed)
  });

  test('intercom board renders (Audio retired, W8)', async ({ page }) => {
    await page.goto('/intercom');
    await expect(page.getByRole('heading', { name: 'Intercom' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Board' })).toBeVisible();
  });

  test('vehicles view renders (reports or empty state)', async ({ page }) => {
    await page.goto('/vehicles');
    await expect(page.locator('h2', { hasText: 'Vehicle Status' })).toBeVisible();
    const hasVehicles = await page.locator('.vehicle-card').count();
    if (hasVehicles === 0) {
      await expect(page.locator('.empty')).toContainText('No vehicle status reports');
    }
  });
});
