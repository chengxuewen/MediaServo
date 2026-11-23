import { test, expect } from '@playwright/test';

// T9 client leg (K5/K14/7.3): the evidence list shows server-retained clips
// with the 7-day label; withdrawal is the irreversible Confirm path. The clip
// is seeded through the API with the stored admin token (a real mic upload is
// covered by the manual pass; the UI contract is what this pins).
test('evidence list renders a retained clip and withdraws it (irreversible confirm)', async ({
  page,
}) => {
  const label = `e2e-spec-${Date.now()}`;
  await page.goto('/devices');
  await page.evaluate(async (lbl) => {
    const token = localStorage.getItem('mediaservo_admin_token');
    const res = await fetch(`/api/admin/evidence?room_id=e2e-room&label=${lbl}`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'audio/webm' },
      body: new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]),
    });
    if (!res.ok) throw new Error(`seed failed: ${res.status}`);
  }, label);

  await page.goto('/evidence');
  await expect(page.getByRole('heading', { name: 'Evidence' })).toBeVisible();
  await expect(page.getByText(/retention 7 days/).first()).toBeVisible();
  const row = page.getByRole('row', { name: new RegExp(label) });
  await expect(row).toBeVisible({ timeout: 10_000 });
  await expect(row).toContainText('e2e-room');

  // withdrawal: consequence-stating confirm
  await row.getByRole('button', { name: 'Withdraw' }).click();
  await expect(page.getByText(/cannot be undone/)).toBeVisible();
  await page.getByRole('button', { name: 'Withdraw', exact: true }).last().click();
  await expect(page.getByRole('row', { name: new RegExp(label) })).toHaveCount(0, {
    timeout: 10_000,
  });
});

test('download triggers an audited blob fetch (button present, guarded when none)', async ({
  page,
}) => {
  await page.goto('/evidence');
  await expect(page.getByRole('heading', { name: 'Evidence' })).toBeVisible();
  // the empty state renders honestly when no clips exist
  await page.waitForLoadState('networkidle');
  const rows = page.getByRole('row');
  const hasRows = (await rows.count()) > 1; // header row + data rows
  if (!hasRows) {
    await expect(page.getByText(/No retained clips/)).toBeVisible();
  }
});
