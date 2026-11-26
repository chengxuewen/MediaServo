import { test, expect } from './host-fixture';
import { openPicker } from './flow';

// B3 picker (V9b K12=A: device list now lives in the Picker) + M1 stale/Retry in the rail.
test.describe.serial('Monitor picker (B3)', () => {
  test('picker opens with search + ARIA tree + keyboard; add to wall', async ({ page, hostProducer }) => {
    await page.goto('/');
    await openPicker(page);
    await expect(page.locator('[role="tree"]')).toBeVisible();
    const search = page.getByLabel('Search streams');
    await expect(search).toBeVisible();
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    // search filters to zero, then back
    await search.fill('zzz-no-match');
    await expect(page.locator('[role="treeitem"]')).toHaveCount(0);
    await search.fill('');
    await expect(page.locator('[role="treeitem"]').first()).toBeVisible();

    // keyboard: focus the tree, Enter toggles the focused item into chosen
    await page.locator('[role="tree"]').focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('.picker-chosen-item')).toHaveCount(1);

    await page.locator('.picker-add').click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
  });

  // V15 geometry guard: both panes must stretch to the SAME bottom (the right list
  // previously ended early and the two columns misaligned). Mock producers so this is
  // deterministic and independent of a live host.
  test('two panes share the same height/bottom (no misalignment)', async ({ page }) => {
    const streams = Array.from({ length: 6 }, (_, i) => ({
      room: 'cam-geo', owner: null, peer_id: `host-${i}`, role: 'producer', id: `g${i}`, kind: 'video',
      byte_count: 1, packet_count: 1, score: 10, transport_id: `t${i}`, local_port: 1,
      remote_ip: '127.0.0.1', remote_port: 1, rtp_packet_loss_sent: 0, rtp_packet_loss_received: 0,
      available_outgoing_bitrate: null, available_incoming_bitrate: 1000000,
    }));
    await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams } }));
    await page.goto('/');
    await openPicker(page);
    await page.locator('.picker-dev .pick-caret').first().click();

    const box = (s: string) => page.locator(s).boundingBox();
    const l = await box('.picker-avail');
    const rr = await box('.picker-chosen');
    expect(l && rr).toBeTruthy();
    expect(Math.abs((l!.height) - (rr!.height))).toBeLessThanOrEqual(2);
    expect(Math.abs((l!.y + l!.height) - (rr!.y + rr!.height))).toBeLessThanOrEqual(2);
    expect(l!.height).toBeGreaterThan(200);
  });

  test('M1 stale banner + Retry keeps last known streams on fetch error', async ({ page, hostProducer }) => {
    await page.goto('/');
    await openPicker(page);
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });
    await page.keyboard.press('Escape'); // back to the rail

    // fail subsequent fetches; the poll surfaces a stale banner but keeps the last data
    await page.route('**/api/admin/sfu/stats', (route) => route.abort());
    await expect(page.locator('.st-stale')).toBeVisible({ timeout: 15000 });
    await expect(page.locator('.st-retry').first()).toBeVisible();

    // Retry after the route heals restores live data (stale banner clears)
    await page.unroute('**/api/admin/sfu/stats');
    await page.locator('.st-retry').first().click();
    await expect(page.locator('.st-stale')).toHaveCount(0, { timeout: 15000 });
  });

// V50: the Selected pane rows follow the prototype .pick-row — "<device> · <b>stream</b>"
// (device secondary ink, stream bold), a status DOT only (no text), 24x24 r4 actions.
test('selected rows read "<device> · <stream>" with a bold stream and a dot-only status', async ({ page }) => {
  const streams = Array.from({ length: 3 }, (_, i) => ({
    room: 'cam-geo', owner: null, peer_id: `host-${i}`, role: 'producer', id: `g${i}`, kind: 'video', label: ['front', 'rear', 'cabin'][i],
    byte_count: 1, packet_count: 1, score: 10, transport_id: `t${i}`, local_port: 1,
    remote_ip: '127.0.0.1', remote_port: 1, rtp_packet_loss_sent: 0, rtp_packet_loss_received: 0,
    available_outgoing_bitrate: null, available_incoming_bitrate: 1000000,
  }));
  await page.route('**/api/admin/sfu/stats', (r) => r.fulfill({ json: { streams } }));
  await page.goto('/');
  await openPicker(page);
  await page.locator('.picker-dev .pick-caret').first().click();
  await page.locator('.picker-child').nth(0).click();
  await page.locator('.picker-child').nth(2).click();

  const rows = page.locator('.picker-chosen-item');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).locator('.pc-name')).toHaveText('cam-geo · front');
  await expect(rows.nth(1).locator('.pc-name')).toHaveText('cam-geo · cabin');
  const stream = rows.nth(0).locator('.pc-stream');
  expect(await stream.evaluate((e) => getComputedStyle(e).fontWeight)).toBe('600');
  // status is a dot only
  const st = rows.nth(0).locator('.pick-status');
  expect((await st.textContent())!.trim()).toBe('');
  await expect(st.locator('.pick-dot')).toBeVisible();
  // 24x24 r4 action buttons (prototype .pick-row .acts button)
  const btn = (await rows.nth(0).locator('.pc-acts button').first().boundingBox())!;
  expect(Math.round(btn.width)).toBe(24);
  expect(Math.round(btn.height)).toBe(24);
  expect(await rows.nth(0).locator('.pc-acts button').first().evaluate((e) => getComputedStyle(e).borderRadius)).toBe('4px');
});
});
