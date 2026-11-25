import { test, expect } from '@playwright/test';
import { openPicker } from './flow';

// V3 (monitor-nvr-visual-parity): left-rail on-wall cards + drag reorder / drag-to-remove
// + narrow overlay-drawer mutual exclusion + pin. Mock /sfu/stats (2 producers) so two
// cards exist without a live host (cards derive from playing, not media).
const prod = (id: string, room: string) => ({
  room, owner: null, peer_id: 'host', role: 'producer', id, kind: 'video',
  byte_count: 1, packet_count: 1, score: 10, transport_id: `t-${id}`, local_port: 1,
  remote_ip: '127.0.0.1', remote_port: 1, rtp_packet_loss_sent: 0, rtp_packet_loss_received: 0,
  available_outgoing_bitrate: null, available_incoming_bitrate: 1000000,
});

test.describe('V3 rail cards + drag + drawer', () => {
  test('drag reorders on-wall cards', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams: [prod('a', 'room-a'), prod('b', 'room-b')] } }));
    await page.goto('/');
    await openPicker(page);
    await page.locator('.st-group', { hasText: 'room-a' }).locator('.st-play-all').click();
    await openPicker(page);
    await page.locator('.st-group', { hasText: 'room-b' }).locator('.st-play-all').click();
    await expect(page.locator('.rail-card')).toHaveCount(2);
    const before = await page.locator('.rc-name').allTextContents();
    await page.locator('.rail-card').nth(1).dragTo(page.locator('.rail-card').nth(0));
    const after = await page.locator('.rc-name').allTextContents();
    expect(after[0]).toBe(before[1]);
  });

  test('drag onto remove zone closes the tile', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams: [prod('a', 'room-a'), prod('b', 'room-b')] } }));
    await page.goto('/');
    await openPicker(page);
    await page.locator('.st-group', { hasText: 'room-a' }).locator('.st-play-all').click();
    await openPicker(page);
    await page.locator('.st-group', { hasText: 'room-b' }).locator('.st-play-all').click();
    await expect(page.locator('.rail-card')).toHaveCount(2);
    await page.locator('.rail-card').first().dragTo(page.locator('.rail-drop-remove'));
    await expect(page.locator('.rail-card')).toHaveCount(1);
  });

  test('narrow drawer mutual exclusion + pin visible', async ({ page }) => {
    // V28: 3 columns stay docked down to 768px; drawers only below that.
    await page.setViewportSize({ width: 700, height: 800 });
    await page.goto('/');
    // narrow defaults both closed; open left -> pin visible (pin lives in the rail head)
    await page.locator('.region-toggles [aria-label="Toggle stream rail"]').click();
    await expect(page.locator('.stream-rail')).toBeVisible();
    await expect(page.locator('.pin-btn')).toBeVisible();
    // opening right closes left (mutual exclusion)
    await page.locator('.region-toggles [aria-label="Toggle alert panel"]').click();
    await expect(page.locator('.mon-alerts')).toBeVisible();
    await expect(page.locator('.stream-rail')).toBeHidden();
  });
});