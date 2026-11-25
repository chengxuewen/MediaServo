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
});