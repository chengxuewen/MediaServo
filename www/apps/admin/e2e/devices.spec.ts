import { test, expect } from '@playwright/test';

// W2 (admin-prototype-restyle, B3 real credentials face): enrollment +
// one-time close-protected secrets + reset/revoke semantics confirms.
// Hub-tab reach (SPA nav) per registered dev quirks.
async function openDevicesTab(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.getByRole('link', { name: 'Admin', exact: true }).click();
  await page.getByRole('tab', { name: 'Devices' }).click();
}

test.describe('Devices credentials lifecycle (W2)', () => {
  test('head + search + pending/registered tables', async ({ page }) => {
    await openDevicesTab(page);
    await expect(page.getByRole('heading', { name: 'Devices', exact: true })).toBeVisible();
    await expect(page.getByPlaceholder('Search devices…')).toBeVisible();
    await expect(page.getByRole('button', { name: /Register device/ })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Pending Devices' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Registered Devices' })).toBeVisible();
    // pending table only exists when the queue is non-empty (honest empty state)
    const pendingTable = page.getByRole('columnheader', { name: 'First seen' });
    const pendingEmpty = page.getByRole('status').filter({ hasText: 'No pending devices' });
    await expect(pendingTable.or(pendingEmpty)).toBeVisible();
  });

  test('register with provided secret -> close protection -> reset -> revoke', async ({ page }) => {
    const id = `ms-${Date.now().toString(16).padStart(12, '0').slice(-12)}`;
    const secret = 'e2e-provided-secret-123';
    await openDevicesTab(page);
    await page.getByRole('button', { name: /Register device/ }).click();
    await page.getByLabel('Device ID').fill(id);
    await page.getByLabel('Secret', { exact: true }).fill(secret);
    await page.getByRole('dialog').getByRole('button', { name: 'Register', exact: true }).click();

    // one-time secret modal shows the PROVIDED secret verbatim
    const secretBox = page.getByRole('dialog').getByLabel('Device secret');
    await expect(secretBox).toHaveValue(secret);
    // close protection: Done without copy -> discard confirm (nested dialog: last)
    await page.getByRole('dialog').last().getByRole('button', { name: 'Done' }).click();
    await expect(page.getByRole('dialog').last().getByText('Discard the one-time secret?')).toBeVisible();
    await page.getByRole('dialog').last().getByRole('button', { name: 'Discard' }).click();
    await expect(page.getByRole('cell', { name: id, exact: true })).toBeVisible();

    // reset goes through a semantics confirm, then a fresh one-time secret
    const row = page.getByRole('row').filter({ hasText: id });
    await row.getByRole('button', { name: 'Reset secret' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Reset', exact: true }).click();
    await expect(page.getByRole('dialog').getByLabel('Device secret')).not.toHaveValue(secret);
    await page.getByRole('dialog').getByRole('button', { name: 'Done' }).click();
    await page.getByRole('dialog').last().getByRole('button', { name: 'Discard' }).click();

    // revoke semantics confirm removes the credential
    await row.getByRole('button', { name: 'Revoke' }).click();
    await expect(page.getByRole('dialog')).toContainText('new connections are refused immediately');
    await page.getByRole('dialog').getByRole('button', { name: 'Revoke', exact: true }).click();
    await expect(page.getByText(`Device ${id} revoked`)).toBeVisible();
    await expect(page.getByRole('cell', { name: id, exact: true })).toHaveCount(0);
  });
});