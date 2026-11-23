import { test, expect } from '@playwright/test';

// W3 (admin-prototype-restyle): Keys surface lives in the Admin hub tab —
// the standalone /apikeys dev route is swallowed by the vite /api proxy, and
// a direct /admin load hits the basename heuristic (renders overview), so
// specs and baselines reach the hub via SPA nav (registered open items).
async function openKeysTab(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.getByRole('link', { name: 'Admin', exact: true }).click();
  await page.getByRole('tab', { name: 'API Keys' }).click();
}

test.describe('Keys surface (W3 prototype language)', () => {
  test('head + search + table language render in the Admin hub', async ({ page }) => {
    await openKeysTab(page);
    await expect(page.getByRole('heading', { name: 'Keys', exact: true })).toBeVisible();
    await expect(page.getByPlaceholder('Search keys…')).toBeVisible();
    await expect(page.getByRole('button', { name: /New key/ })).toBeVisible();
    // honest empty state when the store has no keys (table language is
    // asserted in the round-trip test once a key exists)
    const empty = page.getByRole('status').filter({ hasText: 'No API keys yet' });
    const table = page.getByRole('table');
    await expect(empty.or(table)).toBeVisible();
  });

  test('create -> one-time secret -> search -> revoke round-trip', async ({ page }) => {
    const id = `e2e-key-${Date.now()}`;
    await openKeysTab(page);
    await page.getByRole('button', { name: /New key/ }).click();
    await page.getByLabel('Key ID').fill(id);
    await page.getByRole('dialog').getByRole('button', { name: 'Create', exact: true }).click();
    await expect(page.getByRole('dialog')).toContainText('shown only once');
    await page.getByRole('dialog').getByRole('button', { name: 'Done' }).click();
    await expect(page.getByRole('cell', { name: id, exact: true })).toBeVisible();
    // prototype column subset — no Expires column (backend has no such field)
    for (const h of ['Key', 'Role', 'Device scope', 'Label', 'Actions']) {
      await expect(page.getByRole('columnheader', { name: h, exact: true })).toBeVisible();
    }

    // search narrows to the new key, then to the honest empty state
    await page.getByPlaceholder('Search keys…').fill(id);
    await expect(page.getByRole('cell', { name: id, exact: true })).toBeVisible();
    await page.getByPlaceholder('Search keys…').fill('zzz-no-match');
    await expect(page.getByText('No keys match.')).toBeVisible();
    await page.getByPlaceholder('Search keys…').fill(id);

    // revoke goes through the kit Confirm (no window.confirm)
    await page.getByRole('button', { name: 'Revoke', exact: true }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Revoke', exact: true }).click();
    await expect(page.getByText(`API key ${id} revoked`)).toBeVisible();
    await expect(page.getByRole('cell', { name: id, exact: true })).toHaveCount(0);
  });
});