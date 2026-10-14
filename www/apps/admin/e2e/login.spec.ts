import { test, expect } from '@playwright/test';
import { login } from './helpers';

const TOKEN_KEY = 'mediaservo_admin_token';

// Anonymous suite (playwright.config anon project, no storageState) — route guards + login page + 401 self-heal.
// Prereq: Docker server (9800) + vite dev (5173) + accounts.docker.yaml dev account.
test.describe('Auth Guard', () => {
  test('unauthenticated visit redirects to /login without API spam', async ({ page }) => {
    const apiRequests: string[] = [];
    page.on('request', (req) => {
      if (req.url().includes('/api/admin')) apiRequests.push(req.url());
    });

    await page.goto('/');

    await expect(page).toHaveURL(/\/login$/);
    expect(apiRequests).toEqual([]);
  });

  test('expired/invalid token self-heals: 401 clears token and redirects to login', async ({ page }) => {
    // addInitScript re-runs on every navigation → sessionStorage gate avoids re-injecting after redirect
    await page.addInitScript((key) => {
      if (!sessionStorage.getItem('test-token-injected')) {
        localStorage.setItem(key, 'invalid-token');
        sessionStorage.setItem('test-token-injected', '1');
      }
    }, TOKEN_KEY);

    await page.goto('/');

    await expect(page).toHaveURL(/\/login$/, { timeout: 15000 });
    const token = await page.evaluate((key) => localStorage.getItem(key), TOKEN_KEY);
    expect(token).toBeNull();
  });
});

test.describe('Login Page', () => {
  test('wrong password shows error and stays on login', async ({ page }) => {
    await page.goto('/login');
    await page.locator('input.login-input[placeholder="username"]').fill('admin');
    await page.locator('input.login-input[placeholder="password"]').fill('wrong-password');
    await page.locator('button', { hasText: 'Login' }).click();

    await expect(page.locator('.token-status.error')).toBeVisible();
    await expect(page).toHaveURL(/\/login$/);
    const token = await page.evaluate((key) => localStorage.getItem(key), TOKEN_KEY);
    expect(token).toBeNull();
  });

  test('valid login reaches dashboard and rooms API returns 200', async ({ page }) => {
    const roomsResponse = page.waitForResponse(
      (res) => res.url().includes('/api/admin/rooms') && res.request().method() === 'GET',
    );

    await login(page, { username: 'admin', password: 'admin123', role: 'admin' });

    await expect(page.locator('.dashboard')).toBeVisible();
    const rooms = await roomsResponse;
    expect(rooms.status()).toBe(200);
  });

  test('logout clears token and returns to login', async ({ page }) => {
    await login(page, { username: 'dispatcher', password: 'dispatch123', role: 'dispatcher' });
    await expect(page.locator('.dashboard')).toBeVisible();

    await page.locator('.logout-btn').click();

    await expect(page).toHaveURL(/\/login$/);
    const token = await page.evaluate((key) => localStorage.getItem(key), TOKEN_KEY);
    expect(token).toBeNull();
  });
});
