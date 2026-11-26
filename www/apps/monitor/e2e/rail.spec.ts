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

// V45: rail cards show "<device> · <stream>" (the Produce.label), not just the room.
test('rail card names show device and stream', async ({ page }) => {
  await page.goto('/?demo=1');
  const names = await page.locator('.rail-card .rc-name').allTextContents();
  expect(names.length).toBeGreaterThan(2);
  for (const n of names) expect(n).toMatch(/^VH-\d{2} · (front|rear|cabin)$/);
  // and the full name is not clipped (no ellipsis) inside the narrow rail
  const clipped = await page.evaluate(() => [...document.querySelectorAll('.rail-card .rc-name')]
    .filter((e) => e.scrollWidth > e.clientWidth + 1).length);
  expect(clipped).toBe(0);
});

// V47: selecting a rail card highlights it AND the matching wall tile (prototype
// .stream-item.active / .tile.selected), pages/scrolls to it, and a wall tile click
// selects back into the rail. Clicks target .rc-name (the card is draggable, so a
// centre click can be swallowed by the drag gesture).
test('rail selection highlights card + wall tile and locates it', async ({ page }) => {
  await page.goto('/?demo=1');
  const cards = page.locator('.rail-card');
  await expect(cards.first()).toBeVisible();

  const second = (await cards.nth(1).locator('.rc-name').textContent())!.trim();
  await cards.nth(1).locator('.rc-name').click();
  await expect(page.locator('.rail-card.selected')).toHaveCount(1);
  await expect(page.locator('.rail-card.selected .rc-name')).toHaveText(second);
  const sel = page.locator('.wall-cell.selected');
  await expect(sel).toHaveCount(1);
  const device = second.split(' · ')[0];
  await expect(sel.locator('.vp-title').first()).toContainText(device);
  // the selected tile is brought into the viewport
  const box = (await sel.boundingBox())!;
  const vh = page.viewportSize()!.height;
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(vh + 1);

  // selection moves when another card is clicked
  const third = (await cards.nth(2).locator('.rc-name').textContent())!.trim();
  await cards.nth(2).locator('.rc-name').click();
  await expect(page.locator('.rail-card.selected .rc-name')).toHaveText(third);
  await expect(page.locator('.rail-card.selected')).toHaveCount(1);

  // clicking a wall tile selects the matching rail card
  const cellTitle = (await page.locator('.wall-cell').nth(3).locator('.vp-title').first().textContent())!.trim();
  await page.locator('.wall-cell').nth(3).locator('.vp-title').first().click();
  await expect(page.locator('.rail-card.selected')).toHaveCount(1);
  // the rail card for THAT stream (wall order != rail order) is the selected one
  await expect(page.locator('.rail-card.selected .rc-name')).toHaveText(cellTitle);
});
});
