import { test, expect } from '@playwright/test';

// T13-lite (talk-app layer-1 acceptance): the intercom board renders LIVE floor
// truth from BoardStore over the vite /ws + /api proxy. Prereq: the e2e server
// runs with floor enabled and talk-ops/talk-alpha designated
// (MEDIASERVO_FLOOR_ROOMS / MEDIASERVO_FLOOR_ENABLED) — see the run recipe in
// docs/plans/talk-app §8 (SB-4 entry).
//
// This spec runs in the `admin` project only (admin storageState; the other
// projects pin their own testMatch patterns).

test('admin sees the floor board with both talk rooms', async ({ page }) => {
  await page.goto('/intercom');
  await expect(page.getByRole('heading', { name: 'Intercom' })).toBeVisible();
  await expect(page.getByRole('listitem')).toHaveCount(2);
  await expect(page.getByText('talk-ops')).toBeVisible();
  await expect(page.getByText('talk-alpha')).toBeVisible();
  // floor registration delivers the initial scoped floor_state -> mode shows
  await expect(page.locator('.msr-talk-mode', { hasText: 'exclusive' }).first()).toBeVisible({
    timeout: 10_000,
  });
});

test('admin can request and release the floor on talk-ops (WS round-trip)', async ({ page }) => {
  await page.goto('/intercom');
  const row = page.locator('.msr-talk-row', { hasText: 'talk-ops' });
  await expect(row).toBeVisible();

  const req = row.getByRole('button', { name: 'Request' });
  await expect(req).toBeEnabled({ timeout: 10_000 });
  await req.click();

  // granted -> held (hue+shape glyph label carries the holder)
  await expect(row.locator('.msr-talk-state')).toContainText('held:', { timeout: 10_000 });

  const rel = row.getByRole('button', { name: 'Release' });
  await expect(rel).toBeEnabled();
  await rel.click();

  // released -> idle; Release becomes guarded (aria-disabled + reason)
  await expect(row.locator('.msr-talk-state')).toContainText('idle', { timeout: 10_000 });
  await expect(rel).toHaveAttribute('aria-disabled', 'true');
});

test('unknown state: killing the ws flips rows to UNKNOWN with guarded actions', async ({
  page,
}, testInfo) => {
  // K discipline regression: a down connection is UNKNOWN, never idle.
  // Simulated by blocking the page's ws traffic via context offline.
  await page.goto('/intercom');
  await expect(page.getByRole('listitem').first()).toBeVisible({ timeout: 10_000 });
  await page.context().setOffline(true);
  // BoardStore backoff reconnects; while down the rows must show unknown
  await expect(page.locator('.msr-talk-state', { hasText: 'unknown' }).first()).toBeVisible({
    timeout: 30_000,
  });
  const row = page.locator('.msr-talk-row', { hasText: 'talk-ops' });
  await expect(row.getByRole('button', { name: 'Request' })).toHaveAttribute('aria-disabled', 'true');
  await expect(page.context().browser()).toBeTruthy(); // keep testInfo used minimally
  void testInfo;
});

test('session view opens from a board row and degrades honestly without a mic', async ({ page }) => {
  await page.goto('/intercom');
  const row = page.locator('.msr-talk-row', { hasText: 'talk-ops' });
  await expect(row).toBeVisible();
  await row.click();
  await expect(page.getByRole('button', { name: /HOLD TO TALK/ })).toBeVisible({ timeout: 10_000 });
  // headless chromium has no microphone: the honest cue must say so, and the
  // PTT control stays live (floor control works; the server auto-revokes).
  await expect(page.locator('.msr-error')).toContainText('microphone unavailable', {
    timeout: 10_000,
  });
  await expect(page.getByRole('button', { name: /HOLD TO TALK/ })).toBeEnabled();
  // Esc returns to the board
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Intercom' })).toBeVisible();
});
