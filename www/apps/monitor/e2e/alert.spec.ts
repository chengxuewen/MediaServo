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
    // V9c (K11=A): prototype row language — dot + title + mono time + severity word tag.
    await expect(page.locator('.ap-dot')).toHaveCount(2);
    await expect(page.locator('.ap-title', { hasText: 'Device offline' })).toHaveCount(1);
    await expect(page.locator('.ap-title', { hasText: 'Authentication failure' })).toHaveCount(1);
    await expect(page.locator('.ap-tag', { hasText: 'Alarm' })).toHaveCount(1);
    await expect(page.locator('.ap-tag', { hasText: 'Warn' })).toHaveCount(1);
    await expect(page.locator('.ap-time').first()).toHaveText(/^\d{2}:\d{2}:\d{2}$/);

    await page.locator('.alert-panel .chip', { hasText: 'Alarm' }).click();
    await expect(page.locator('.ap-row')).toHaveCount(1);
    await page.locator('.alert-panel .chip', { hasText: 'All' }).click();
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

  test('auto-wall plays a NEW danger alert room when armed (B1 behavior)', async ({ page, hostProducer }) => {
    let auditEvents: Array<Record<string, unknown>> = [];
    await page.route('**/api/admin/audit', (r) => r.fulfill({
      json: { events: auditEvents, count: auditEvents.length, source: 'core', retention: 'memory' },
    }));
    await page.goto('/');
    
    await expect(page.locator('.alert-panel')).toBeVisible({ timeout: 15000 });
    // arm first (no alerts yet), then inject a NEW estop alert for the live host room
    await page.locator('.ap-autowall').click();
    auditEvents = [{ kind: 'estop_audit', ts: 1700000000, room_id: hostProducer.room }];
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
  });

  test('right alerts default OPEN (prototype) + playback transport guarded (K7)', async ({ page }) => {
    // V24: clear the persisted region memory so the default (playback OFF) is exercised.
    await page.addInitScript(() => { try { localStorage.removeItem('mediaservo_monitor_regions'); } catch { /* ignore */ } });
    await page.goto('/');
    await expect(page.locator('.mon-alerts')).toBeVisible();
    await expect(page.locator('.bottom-zone')).toBeHidden();
    await page.locator('[aria-label="Toggle playback bar"]').click();
    await expect(page.locator('.ctl-bar')).toBeVisible();
    const guarded = page.locator('.ctl-bar [aria-disabled="true"]');
    expect(await guarded.count()).toBeGreaterThan(5);
    await expect(guarded.first()).toHaveAttribute('data-reason', /RECORDING_READY=false/);
  });
});