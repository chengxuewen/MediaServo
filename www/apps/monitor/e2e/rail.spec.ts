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
  // V49: live tile selection ring = top overlay 2px accent border
  const liveRing = await sel.evaluate((e) => {
    const a = getComputedStyle(e, '::after');
    return `${a.borderTopWidth} ${a.borderTopStyle} ${a.borderTopColor} z=${a.zIndex}`;
  });
  expect(liveRing).toBe('2px solid rgb(67, 97, 238) z=30');
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

// V48: prototype .stream-item — cards are sunken GREY by default; only the selected
// card is accent-tinted with an accent border (being LIVE is not selection).
test('rail cards default to grey; only the selected card is blue', async ({ page }) => {
  await page.goto('/?demo=1');
  const cards = page.locator('.rail-card');
  await expect(cards.first()).toBeVisible();
  const before = await cards.evaluateAll((els) => els.map((e) => {
    const c = getComputedStyle(e);
    return { bg: c.backgroundColor, bd: c.borderColor };
  }));
  for (const c of before) {
    expect(c.bg).toBe('rgb(233, 237, 243)'); // --sunken
    expect(c.bd).toBe('rgb(228, 232, 239)'); // --bd
  }
  await cards.nth(1).locator('.rc-name').click();
  const sel = page.locator('.rail-card.selected');
  await expect(sel).toHaveCount(1);
  const cs = await sel.evaluate((e) => { const c = getComputedStyle(e); return { bg: c.backgroundColor, bd: c.borderColor }; });
  expect(cs.bg).toBe('rgb(238, 241, 255)'); // --accent-tint
  expect(cs.bd).toBe('rgb(67, 97, 238)'); // --accent
  // the other cards stay grey
  const others = await page.locator('.rail-card:not(.selected)').evaluateAll((els) => els.map((e) => getComputedStyle(e).backgroundColor));
  for (const bg of others) expect(bg).toBe('rgb(233, 237, 243)');
});

// V48: selecting a card that lives on ANOTHER wall page flips to that page and the
// selected tile is on screen (prototype gotoStream).
test('selecting a stream on another page flips the pager and shows the tile', async ({ page }) => {
  await page.goto('/?demo=1');
  await page.locator('.layout-menu').click();
  await page.getByRole('option', { name: '4', exact: true }).click();
  await expect(page.locator('.wt-page')).toHaveText('1 / 2');
  // the 6th card (index 5) is on page 2
  await page.locator('.rail-card').nth(5).locator('.rc-name').click();
  await expect(page.locator('.wt-page')).toHaveText('2 / 2');
  const sel = page.locator('.wall-cell.selected, .wall-offline.selected');
  await expect(sel).toHaveCount(1);
  // V49: the ring is a TOP overlay (::after), not an inset shadow hidden under the tile
  const ring = await sel.evaluate((e) => {
    const a = getComputedStyle(e, '::after');
    return { bw: a.borderTopWidth, bs: a.borderTopStyle, bc: a.borderTopColor, pos: a.position, z: parseInt(a.zIndex, 10) };
  });
  expect(ring.bw).toBe('2px');
  expect(ring.bs).toBe('solid');
  expect(ring.bc).toBe('rgb(67, 97, 238)');
  expect(ring.pos).toBe('absolute');
  expect(ring.z).toBeGreaterThanOrEqual(20);
  const box = (await sel.boundingBox())!;
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(page.viewportSize()!.height + 1);
  // back to a page-1 stream
  await page.locator('.rail-card').nth(0).locator('.rc-name').click();
  await expect(page.locator('.wt-page')).toHaveText('1 / 2');
  await expect(page.locator('.wall-cell.selected, .wall-offline.selected')).toHaveCount(1);
});
});
