import { test, expect } from './host-fixture';

// B4 (monitor-prototype-parity): right-rail alert panel — alert-worthy audit subset
// (restyle:256 danger/warn), filter chips, unread badge, click-to-wall, auto-wall arm.
// Alert events are injected by mocking /api/admin/audit (deterministic; real danger
// kinds originate from WS signaling / device disconnect, not a REST POST — O1).
const MOCK_AUDIT = {
  events: [
    { kind: 'device_offline', ts: 1700000000, room_id: 'cam-x' },
    { kind: 'auth_failure', ts: 1700000001, username: 'intruder' },
  ],
  count: 2, source: 'core', retention: 'memory',
};

test.describe('Monitor alert panel (B4)', () => {
  test('filter chips + unread badge + filter + click marks read', async ({ page }) => {
    await page.route('**/api/admin/audit', (r) => r.fulfill({ json: MOCK_AUDIT }));
    await page.goto('/');
    await expect(page.locator('.alert-panel')).toBeVisible({ timeout: 15000 });
    await expect(page.locator('.alert-panel .chip')).toHaveCount(3);
    await expect(page.locator('.ap-row')).toHaveCount(2);
    await expect(page.locator('.ap-unread')).toHaveText('2');

    await page.locator('.alert-panel .chip', { hasText: 'danger' }).click();
    await expect(page.locator('.ap-row')).toHaveCount(1);
    await page.locator('.alert-panel .chip', { hasText: 'all' }).click();
    await expect(page.locator('.ap-row')).toHaveCount(2);

    await page.locator('.ap-row').first().click();
    await expect(page.locator('.ap-unread')).toHaveText('1');
  });

  test('auto-wall toggle arms (aria-pressed)', async ({ page }) => {
    await page.route('**/api/admin/audit', (r) => r.fulfill({ json: MOCK_AUDIT }));
    await page.goto('/');
    await expect(page.locator('.alert-panel')).toBeVisible({ timeout: 15000 });
    const aw = page.locator('.ap-autowall');
    await expect(aw).toHaveAttribute('aria-pressed', 'false');
    await aw.click();
    await expect(aw).toHaveAttribute('aria-pressed', 'true');
  });

  test('click alert jumps to wall (plays the alert room)', async ({ page, hostProducer }) => {
    await page.route('**/api/admin/audit', (r) => r.fulfill({
      json: { events: [{ kind: 'device_offline', ts: 1700000000, room_id: hostProducer.room }], count: 1, source: 'core', retention: 'memory' },
    }));
    await page.goto('/');
    await expect(page.locator('.ap-row')).toHaveCount(1, { timeout: 15000 });
    await page.locator('.ap-row').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
  });
});