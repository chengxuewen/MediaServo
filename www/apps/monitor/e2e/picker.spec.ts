import { test, expect } from './host-fixture';

// B3 (monitor-prototype-parity): add-stream shuttle picker (search + role=tree +
// arrow/Enter keyboard, B5 contract) and M1 stale-data + Retry. Serial: one host
// fixture at a time. The ARIA tree lives in the kit Modal portal (asserted globally).
test.describe.serial('Monitor picker (B3)', () => {
  test('picker opens with search + ARIA tree + keyboard; add to wall', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    await page.locator('.open-picker').click();
    await expect(page.locator('[role="tree"]')).toBeVisible();
    await expect(page.locator('.picker-search')).toBeVisible();

    // search filters to zero, then back
    await page.locator('.picker-search').fill('zzz-no-match');
    await expect(page.locator('[role="treeitem"]')).toHaveCount(0);
    await page.locator('.picker-search').fill('');
    await expect(page.locator('[role="treeitem"]').first()).toBeVisible();

    // keyboard: focus the tree, Enter toggles the focused item into chosen
    await page.locator('[role="tree"]').focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('.picker-chosen-item')).toHaveCount(1);

    await page.locator('.picker-add').click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
  });

  test('M1 stale banner + Retry keeps last known streams on fetch error', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    // fail subsequent fetches; the poll surfaces a stale banner but keeps the tree
    await page.route('**/api/admin/sfu/stats', (route) => route.abort());
    await expect(page.locator('.st-stale')).toBeVisible({ timeout: 15000 });
    await expect(page.locator('.st-retry').first()).toBeVisible();
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room);

    // Retry after the route heals restores live data (stale banner clears)
    await page.unroute('**/api/admin/sfu/stats');
    await page.locator('.st-retry').first().click();
    await expect(page.locator('.st-stale')).toHaveCount(0, { timeout: 15000 });
  });
});