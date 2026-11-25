import { test, expect } from '@playwright/test';
import { openPicker } from './flow';

// V6 (monitor-nvr-visual-parity): elastic/custom layouts — 25/36 empty-slot parity
// (cap16), 2x2 span, column-weight drag, region drag-box zoom + double-click reset.
const prod = (id: string, room: string) => ({
  room, owner: null, peer_id: 'host', role: 'producer', id, kind: 'video',
  byte_count: 1, packet_count: 1, score: 10, transport_id: `t-${id}`, local_port: 1,
  remote_ip: '127.0.0.1', remote_port: 1, rtp_packet_loss_sent: 0, rtp_packet_loss_received: 0,
  available_outgoing_bitrate: null, available_incoming_bitrate: 1000000,
});
const pick = async (page: any, name: string) => { await page.locator('.layout-menu').click(); await page.getByRole('option', { name, exact: true }).click(); };

test.describe('V6 elastic/custom + region zoom', () => {
  test('25-layout fills empty slots under cap16 (K10)', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams: [prod('a', 'room-a'), prod('b', 'room-b')] } }));
    await page.goto('/');
    await openPicker(page);
    await page.locator('.st-group', { hasText: 'room-a' }).locator('.st-play-all').click();
    await openPicker(page);
    await page.locator('.st-group', { hasText: 'room-b' }).locator('.st-play-all').click();
    await pick(page, '25');
    await expect(page.locator('.tile-empty')).toHaveCount(23);
  });

  test('custom 2x2 span toggle persists', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams: [prod('a', 'room-a')] } }));
    await page.goto('/');
    await openPicker(page);
    await page.locator('.st-play-all').first().click();
    await pick(page, 'Custom');
    await page.locator('.wall-grid .video-player').first().hover();
    await page.locator('[aria-label="Toggle 2x2 span"]').click();
    await expect(page.locator('.wall-cell').first()).toHaveCSS('grid-column-start', 'span 2');
  });

  test('column-weight drag changes fr weights', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams: [prod('a', 'room-a')] } }));
    await page.goto('/');
    await openPicker(page);
    await page.locator('.st-play-all').first().click();
    await pick(page, 'Custom');
    const before = await page.locator('.wall-grid').evaluate((el) => getComputedStyle(el).gridTemplateColumns);
    const h = await page.locator('.col-handle').first().boundingBox();
    await page.mouse.move(h!.x + 6, h!.y + 20); await page.mouse.down();
    await page.mouse.move(h!.x + 60, h!.y + 20, { steps: 5 }); await page.mouse.up();
    const after = await page.locator('.wall-grid').evaluate((el) => getComputedStyle(el).gridTemplateColumns);
    expect(after).not.toBe(before);
  });

  test('region drag-box zooms; double-click resets', async ({ page }) => {
    await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams: [prod('a', 'room-a')] } }));
    await page.goto('/');
    await openPicker(page);
    await page.locator('.st-play-all').first().click();
    const body = page.locator('.wall-grid .vp-body').first();
    await expect(body).toBeVisible();
    const b = await body.boundingBox();
    await page.mouse.move(b!.x + b!.width / 2 - 40, b!.y + b!.height / 2 - 30);
    await page.mouse.down(); await page.mouse.move(b!.x + b!.width / 2 + 40, b!.y + b!.height / 2 + 30, { steps: 4 }); await page.mouse.up();
    const t = await page.locator('.wall-grid video').first().evaluate((v) => (v as HTMLVideoElement).style.transform);
    expect(t).toContain('scale');
    await body.dblclick();
    const t2 = await page.locator('.wall-grid video').first().evaluate((v) => (v as HTMLVideoElement).style.transform);
    expect(t2).not.toContain('scale');
  });
});