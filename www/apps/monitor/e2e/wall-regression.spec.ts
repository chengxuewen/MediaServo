import { test, expect } from '@playwright/test';
import { openPicker } from './flow';

// B1/B2 regression (P3 review): deterministic via a /sfu/stats mock. V9b (K12=A): the
// device list lives in the Picker; per-stream checkbox selection was retired (not in the
// prototype), so B1 now asserts per-room Play All is independent (no cross-room no-op).
function producer(id: string, room: string) {
  return {
    room, owner: null, peer_id: 'host', role: 'producer', id, kind: 'video',
    byte_count: 1000, packet_count: 10, score: 10, transport_id: `t-${id}`,
    local_port: 20000, remote_ip: '127.0.0.1', remote_port: 40000,
    rtp_packet_loss_sent: 0, rtp_packet_loss_received: 0,
    available_outgoing_bitrate: null, available_incoming_bitrate: 1000000,
  };
}

test.describe('Monitor wall regressions (B1/B2)', () => {
  test('B1: room B Play All is independent of room A', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (route) => route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ streams: [producer('pa', 'room-a'), producer('pb', 'room-b')] }),
    }));
    await page.goto('/');
    await openPicker(page);
    await expect(page.locator('.st-device-name', { hasText: 'room-a' })).toBeVisible({ timeout: 15000 });
    await expect(page.locator('.st-device-name', { hasText: 'room-b' })).toBeVisible();
    await page.locator('.st-group', { hasText: 'room-b' }).locator('.st-play-all').click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
    await expect(page.locator('.wall-grid .vp-title')).toHaveText('room-b');
  });

  test('B2: a vanished producer frees its cap slot (grace reconcile)', async ({ page }) => {
    let streams = [producer('px', 'room-x')];
    await page.route('**/api/admin/sfu/stats', (route) => route.fulfill({
      contentType: 'application/json', body: JSON.stringify({ streams }),
    }));
    await page.goto('/');
    await openPicker(page);
    await expect(page.locator('.st-device-name', { hasText: 'room-x' })).toBeVisible({ timeout: 15000 });
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });

    streams = [];
    await expect(page.locator('.wall-offline')).toHaveCount(1, { timeout: 10000 });
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(0, { timeout: 15000 });
    await expect(page.locator('.wall-offline')).toHaveCount(0, { timeout: 15000 });
  });

  test('cap16 + >8 confirm: Play All 17 streams caps at 16 with skipped hint (O1/O5)', async ({ page }) => {
    const streams = Array.from({ length: 17 }, (_, i) => producer(`p${i}`, 'room-cap'));
    await page.route('**/api/admin/sfu/stats', (route) => route.fulfill({
      contentType: 'application/json', body: JSON.stringify({ streams }),
    }));
    await page.goto('/');
    await openPicker(page);
    await expect(page.locator('.st-device-name', { hasText: 'room-cap' })).toBeVisible({ timeout: 15000 });
    await page.locator('.st-group', { hasText: 'room-cap' }).locator('.st-play-all').click();
    await page.getByRole('button', { name: 'Play', exact: true }).click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(16, { timeout: 15000 });
    await expect(page.locator('.wt-hint')).toContainText('skipped 1');
  });

  test('layout key migration from legacy cols (R1)', async ({ page }) => {
    await page.addInitScript(() => { localStorage.setItem('mediaservo_monitor_cols', '3'); });
    await page.goto('/');
    await expect(page.locator('.layout-menu')).toContainText('9', { timeout: 15000 });
  });
});