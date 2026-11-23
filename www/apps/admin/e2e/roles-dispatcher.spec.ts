import { test, expect } from '@playwright/test';

// H3: dispatcher role read-only view (storageState signed in as dispatcher — global-setup).
// Asserts: audio/vehicles visible + admin-only REST rejected (server-enforced) + audio panel renders.

test.describe('H3 dispatcher role-aware views', () => {
  test('dispatcher sees audio + vehicles nav, header shows role', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.header .version')).toContainText('dispatcher [dispatcher]');
    // W8 retired: Audio merged into Intercom; Rooms/Evidence are Intercom tabs
    await page.locator('nav a', { hasText: 'Intercom' }).click();
    await expect(page.getByRole('tab', { name: 'Rooms' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Evidence' })).toBeVisible();
    // D12=A: Fleet moved into the Admin hub (one Administration entry)
    await page.locator('nav a', { hasText: 'Admin' }).click();
    await expect(page.getByRole('tab', { name: 'Fleet' })).toBeVisible();
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

  test('intercom board renders (Audio retired, W8)', async ({ page }) => {
    await page.goto('/intercom');
    await expect(page.getByRole('heading', { name: 'Intercom' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Board' })).toBeVisible();
  });
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
