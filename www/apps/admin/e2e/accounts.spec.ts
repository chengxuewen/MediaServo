import { test, expect } from '@playwright/test';

// W6 (admin-prototype-restyle, B1 real face): Accounts via the Admin hub tab
// (SPA nav — direct /admin load hits the basename heuristic, registered quirk).
async function openAccountsTab(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.getByRole('link', { name: 'Admin', exact: true }).click();
  await page.getByRole('tab', { name: 'Accounts' }).click();
}

test.describe('Accounts surface (W6 prototype language)', () => {
  test('head + search + table + honest capability index', async ({ page }) => {
    await openAccountsTab(page);
    await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
    await expect(page.getByPlaceholder('Search accounts…')).toBeVisible();
    await expect(page.getByRole('button', { name: /New account/ })).toBeVisible();
    for (const h of ['Username', 'Role', 'Device scope', 'Actions']) {
      await expect(page.getByRole('columnheader', { name: h, exact: true })).toBeVisible();
    }
    await expect(page.getByRole('heading', { name: 'Capability index' })).toBeVisible();
    // honest absent faces, not simulated provider machinery
    await expect(page.getByRole('cell', { name: 'not in this build' }).first()).toBeVisible();
    await expect(page.getByText('API live; page pending board B6')).toBeVisible();
  });

  test('create -> scope truth -> edit role -> delete round-trip', async ({ page }) => {
    const name = `e2e-acc-${Date.now()}`;
    await openAccountsTab(page);
    await page.getByRole('button', { name: /New account/ }).click();
    await page.getByLabel('Username').fill(name);
    await page.getByLabel('Password', { exact: true }).fill('e2e-pass-123');
    await page.getByRole('dialog').getByRole('button', { name: 'Create', exact: true }).click();
    await expect(page.getByRole('cell', { name, exact: true })).toBeVisible();
    // server truth: viewer with empty allowlist = NO devices (roles.rs can_pull)
    const row = page.getByRole('row').filter({ hasText: name });
    await expect(row.getByText('no devices', { exact: true })).toBeVisible();

    // edit role to dispatcher -> scope becomes all devices
    await row.getByRole('button', { name: 'Edit' }).click();
    await page.getByRole('dialog').getByLabel('Role').click();
    await page.getByRole('option', { name: 'dispatcher' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
    await expect(row.getByText('— (all devices)')).toBeVisible();

    // delete via kit Confirm
    await row.getByRole('button', { name: 'Delete' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(page.getByText(`Account ${name} deleted`)).toBeVisible();
    await expect(page.getByRole('cell', { name, exact: true })).toHaveCount(0);
  });
});