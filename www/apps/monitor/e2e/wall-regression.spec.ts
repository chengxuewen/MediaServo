import { test, expect } from '@playwright/test';

// B1/B2 regression (P3 independent review findings): deterministic via a
// /api/admin/sfu/stats route mock — no real host needed. Pins the two Blocking
// fixes so they cannot regress silently (the single-host wall fixture cannot
// reach multi-room selection or producer-death scenarios).
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
  test('B1: selecting in room A does not silence room B Play All', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (route) => route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ streams: [producer('pa', 'room-a'), producer('pb', 'room-b')] }),
    }));
    await page.goto('/');
    await expect(page.locator('.st-device-name', { hasText: 'room-a' })).toBeVisible({ timeout: 15000 });
    await expect(page.locator('.st-device-name', { hasText: 'room-b' })).toBeVisible();

    // select room A's stream → global selected.size > 0
    await page.locator('.st-group', { hasText: 'room-a' }).locator('.st-check input').check();

    // room B button is per-room "Play All"; clicking it must play room B (not no-op)
    const roomB = page.locator('.st-group', { hasText: 'room-b' });
    await expect(roomB.locator('.st-play-all')).toContainText('Play All');
    await roomB.locator('.st-play-all').click();

    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
    await expect(page.locator('.wall-grid .vp-title')).toHaveText('room-b');
  });

  test('B2: a vanished producer frees its cap slot (grace reconcile)', async ({ page }) => {
    let streams = [producer('px', 'room-x')];
    await page.route('**/api/admin/sfu/stats', (route) => route.fulfill({
      contentType: 'application/json', body: JSON.stringify({ streams }),
    }));
    await page.goto('/');
    await expect(page.locator('.st-device-name', { hasText: 'room-x' })).toBeVisible({ timeout: 15000 });
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });

    // producer dies (device offline / restart uuid churn)
    streams = [];
    // closable offline placeholder appears, then the grace reconcile frees the slot
    await expect(page.locator('.wall-offline')).toHaveCount(1, { timeout: 10000 });
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(0, { timeout: 15000 });
    await expect(page.locator('.wall-offline')).toHaveCount(0, { timeout: 15000 });
  });
});