import { expect, type Page } from '@playwright/test';

// V9b (K12=A): the device tree lives in the Picker modal now; the left rail shows only
// selected cards. These helpers open the picker before using the tree selectors so the
// e2e keeps its .st-play-all / .st-device-name vocabulary.
export async function openPicker(page: Page) {
  await page.locator('.open-picker').click();
  await expect(page.locator('.picker-modal')).toBeVisible();
}
export async function readyProducers(page: Page, room: string) {
  await openPicker(page);
  await expect(page.locator('.st-device-name', { hasText: room })).toBeVisible({ timeout: 15000 });
}
export async function playAllRoom(page: Page, room: string) {
  await openPicker(page);
  await page.locator('.st-group', { hasText: room }).locator('.st-play-all').click();
}
export async function playAllFirst(page: Page) {
  await openPicker(page);
  await page.locator('.st-play-all').first().click();
}