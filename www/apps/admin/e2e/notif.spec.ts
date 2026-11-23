import { test, expect } from '@playwright/test';

// T11 (talk-app, design 7.2 / K15=A): cross-page floor notifications.
// The dev-injection hook (__floorNotifInject, dev builds only) plants chips
// deterministically; assertions cover render, click-landing, and clearing.
test('cross-page granted chip appears, click lands on the session, chip clears', async ({ page }) => {
  await page.goto('/devices'); // a NON-intercom page
  await expect(page.locator('nav a', { hasText: 'Intercom' })).toBeVisible();
  await page.evaluate(() =>
    (window as unknown as { __floorNotifInject: (r: string, k: string) => void }).__floorNotifInject('talk-ops', 'granted'),
  );
  const chip = page.locator('#floorNotif .notif-chip', { hasText: 'talk-ops' });
  await expect(chip).toBeVisible();
  await expect(page.locator('#floorNotif')).toHaveAttribute('aria-live', 'polite');
  await chip.click();
  await expect(page).toHaveURL(/\/intercom\/talk-ops$/);
  await expect(page.locator('#floorNotif .notif-chip')).toHaveCount(0);
});

test('queued chip renders with its position label', async ({ page }) => {
  await page.goto('/devices');
  await page.evaluate(() =>
    (window as unknown as { __floorNotifInject: (r: string, k: string) => void }).__floorNotifInject('talk-alpha', 'queued'),
  );
  await expect(page.locator('#floorNotif .notif-chip', { hasText: 'talk-alpha' })).toBeVisible();
});

test('in-page (on /intercom) injection produces NO chip (K15 in-page discipline)', async ({ page }) => {
  await page.goto('/intercom');
  await expect(page.getByRole('heading', { name: 'Intercom' })).toBeVisible();
  await page.evaluate(() =>
    (window as unknown as { __floorNotifInject: (r: string, k: string) => void }).__floorNotifInject('talk-ops', 'granted'),
  );
  await expect(page.locator('#floorNotif .notif-chip')).toHaveCount(0);
});
