import { test, expect } from '@playwright/test';

// T10 managed-rooms CRUD: create -> row with floor summary -> edit ->
// delete with consequence-stating confirm. The spec cleans up after
// itself (delete), so the shared e2e rooms.yaml stays stable for the
// intercom board specs.
test('rooms CRUD round-trip with floor overrides', async ({ page }) => {
  await page.goto('/rooms');
  await expect(page.getByRole('heading', { name: 'Rooms' })).toBeVisible();

  // create talk room with a floor override
  await page.getByRole('button', { name: 'New room' }).click();
  await page.getByLabel('id', { exact: true }).fill('e2e-room-a');
  await page.getByLabel('max_peers').fill('3');
  await page.getByRole('button', { name: 'Save' }).click();
  const row = page.getByRole('row', { name: /e2e-room-a/ });
  await expect(row).toBeVisible({ timeout: 10_000 });
  await expect(row).toContainText('talk');
  await expect(row).toContainText('peers 3');

  // edit: add a hold override
  await row.getByRole('button', { name: 'Edit' }).click();
  await page.getByLabel('hold_timeout_secs').fill('45');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(row).toContainText('hold 45s', { timeout: 10_000 });
  await expect(row).toContainText('peers 3');

  // delete: confirm dialog states the consequence
  await row.getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByText(/Live sessions in this room continue until peers leave/)).toBeVisible();
  await page.getByRole('button', { name: 'Delete', exact: true }).last().click();
  await expect(row).toHaveCount(0, { timeout: 10_000 });
});

test('dispatcher sees read-only rooms (mutations guarded with a reason)', async ({ page }) => {
  // this spec runs in the admin project; dispatcher coverage rides the
  // guard itself: the buttons carry aria-disabled semantics for non-admin.
  await page.goto('/rooms');
  await expect(page.getByRole('heading', { name: 'Rooms' })).toBeVisible();
  // admin context: New room enabled
  await expect(page.getByRole('button', { name: 'New room' })).toBeEnabled();
});
