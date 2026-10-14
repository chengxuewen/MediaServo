import { test, expect } from '@playwright/test';

// H3: dispatcher role read-only view (storageState signed in as dispatcher — global-setup).
// Asserts: audio/vehicles visible + admin-only REST rejected (server-enforced) + audio panel renders.

test.describe('H3 dispatcher role-aware views', () => {
  test('dispatcher sees audio + vehicles nav, header shows role', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.header .version')).toContainText('dispatcher [dispatcher]');
    await expect(page.locator('nav a', { hasText: 'Audio Conference' })).toBeVisible();
    await expect(page.locator('nav a', { hasText: 'Vehicles' })).toBeVisible();
  });

  test('dispatcher admin-only REST denied, read-only allowed', async ({ page }) => {
    await page.goto('/');
    const token = await page.evaluate(() => localStorage.getItem('mediaservo_admin_token'));
    // config = admin only
    const configStatus = await page.evaluate(async (tok) => {
      const res = await fetch('/api/admin/config', { headers: { Authorization: `Bearer ${tok}` } });
      return res.status;
    }, token);
    expect(configStatus).toBe(401);
    // write op (delete room) rejected
    const deleteStatus = await page.evaluate(async (tok) => {
      const res = await fetch('/api/admin/rooms/nope', { method: 'DELETE', headers: { Authorization: `Bearer ${tok}` } });
      return res.status;
    }, token);
    expect(deleteStatus).toBe(401);
    // read-only endpoint allowed
    const statusStatus = await page.evaluate(async (tok) => {
      const res = await fetch('/api/admin/status', { headers: { Authorization: `Bearer ${tok}` } });
      return res.status;
    }, token);
    expect(statusStatus).toBe(200);
  });

  test('dispatcher audio panel renders (rooms or empty state)', async ({ page }) => {
    await page.goto('/audio');
    await expect(page.locator('h2', { hasText: 'Audio Conference Rooms' })).toBeVisible();
    const hasRooms = await page.locator('.audio-room').count();
    if (hasRooms === 0) {
      await expect(page.locator('.empty')).toContainText('No active audio rooms');
    }
  });
});
