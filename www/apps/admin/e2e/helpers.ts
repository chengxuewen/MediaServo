import { expect, type Page } from '@playwright/test';

export interface Creds { username: string; password: string; role: string; }

/**
 * Sign in + assert signed in. The login endpoint is rate-limited by tower_governor (2 req/s burst 5, G3) —
 * on 429 wait for the bucket refill and retry. Polls localStorage for success (more robust than indicator timing).
 */
export async function login(page: Page, creds: Creds, attempts = 4): Promise<void> {
  for (let i = 0; i < attempts; i++) {
    await page.goto('http://localhost:5173/login');
    await page.locator('input.login-input[placeholder="username"]').fill(creds.username);
    await page.locator('input.login-input[placeholder="password"]').fill(creds.password);
    await page.locator('button', { hasText: 'Login' }).click();

    // Poll: token in localStorage = success; error indicator = failure (rate-limit/credentials) → retry
    for (let t = 0; t < 40; t++) {
      const token = await page.evaluate(() => localStorage.getItem('mediaservo_admin_token'));
      if (token) {
        await expect(page.locator('.header .version')).toContainText(`[${creds.role}]`);
        return;
      }
      const errorCount = await page.locator('.token-status.error').count();
      if (errorCount > 0) break;
      await page.waitForTimeout(150);
    }
    await page.waitForTimeout(1200); // wait for the rate-limit bucket to refill
  }
  throw new Error(`login failed for ${creds.username} after ${attempts} attempts`);
}
