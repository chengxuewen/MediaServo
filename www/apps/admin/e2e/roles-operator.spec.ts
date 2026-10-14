import { test, expect } from '@playwright/test';

// I1 review regression: operator (G3 can_status but no admin REST access) must not see the monitoring nav —
// frontend canMonitor must match server auth_middleware admission (admin|dispatcher).

test.describe('I1 operator admission alignment', () => {
  test('operator does NOT see audio/vehicles nav, header shows role', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.header .version')).toContainText('operator [operator]');
    await expect(page.locator('nav a', { hasText: 'Audio Conference' })).toHaveCount(0);
    await expect(page.locator('nav a', { hasText: 'Vehicles' })).toHaveCount(0);
    // Dashboard data goes through admin REST → operator is server-rejected → 401 error state (admission parity)
    await expect(page.locator('.error')).toContainText('Authentication required');
  });

  test('operator admin REST denied (server admission)', async ({ page }) => {
    await page.goto('/');
    const token = await page.evaluate(() => localStorage.getItem('mediaservo_admin_token'));
    const statusStatus = await page.evaluate(async (tok) => {
      const res = await fetch('/api/admin/status', { headers: { Authorization: `Bearer ${tok}` } });
      return res.status;
    }, token);
    expect(statusStatus).toBe(401);
  });
});
