import { test, expect } from '@playwright/test';
import { login } from './helpers';

const TOKEN_KEY = 'mediaservo_admin_token';

// Anonymous suite (monitor-anon project, no storageState) — route guards + login
// page + token self-heal. Prereq: source server (9800) + vite dev (5176) +
// /tmp/ms-e2e dev accounts.
test.describe('Monitor Auth Guard', () => {
  test('unauthenticated visit redirects to /login without API spam', async ({ page }) => {
    const apiRequests: string[] = [];
    page.on('request', (req) => {
      if (req.url().includes('/api/admin')) apiRequests.push(req.url());
    });

    await page.goto('/');

    await expect(page).toHaveURL(/\/login$/);
    expect(apiRequests).toEqual([]);
  });

  test('expired/invalid token self-heals: guard clears token and redirects to login', async ({ page }) => {
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

test.describe('Monitor Login Page', () => {
  test('wrong password shows error and stays on login', async ({ page }) => {
    await page.goto('/login');
    await page.getByPlaceholder('username').fill('admin');
    await page.getByPlaceholder('password').fill('wrong-password');
    await page.getByRole('button', { name: 'Login', exact: true }).click();

    await expect(page.getByRole('alert')).toBeVisible();
    await expect(page).toHaveURL(/\/login$/);
    const token = await page.evaluate((key) => localStorage.getItem(key), TOKEN_KEY);
    expect(token).toBeNull();
  });

  test('valid login reaches the monitor home shell', async ({ page }) => {
    await login(page, { username: 'admin', password: 'admin123', role: 'admin' });

    await expect(page.locator('.monitor-home')).toBeVisible();
    await expect(page.locator('.header .version')).toContainText('[admin]');
  });

  test('logout clears token and returns to login', async ({ page }) => {
    await login(page, { username: 'dispatcher', password: 'dispatch123', role: 'dispatcher' });
    await expect(page.locator('.monitor-home')).toBeVisible();

    await page.locator('.logout-btn').click();

    await expect(page).toHaveURL(/\/login$/);
    const token = await page.evaluate((key) => localStorage.getItem(key), TOKEN_KEY);
    expect(token).toBeNull();
  });
});