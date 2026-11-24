import { test, expect } from '@playwright/test';

// S13 (P3 slice ③c): the realtime timeline grows as audited events land.
// Trigger = create a uniquely-named managed room → room_config_changed audit event
// carrying that room_id. Growth proof = the unique entry is absent before and present
// after — robust to the ring cap (256), where a raw count delta cannot grow once the
// ring is full (oldest evicted). Cleanup deletes the room (no config residue).
test.describe('Monitor timeline (S13, slice ③c)', () => {
  test('a new audit event appears in the timeline (entries grow)', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.tl-entry').first()).toBeVisible({ timeout: 15000 });

    const roomId = `tl-e2e-${Date.now()}`;
    const newEntry = page.locator('.tl-entry', { hasText: roomId });
    await expect(newEntry).toHaveCount(0);

    const token = await page.evaluate(() => localStorage.getItem('mediaservo_admin_token'));
    const headers = { Authorization: `Bearer ${token}` };
    await page.request.post('/api/admin/managed-rooms', { headers, data: { id: roomId, kind: 'talk' } });
    try {
      await expect(newEntry.first()).toBeVisible({ timeout: 15000 });
    } finally {
      await page.request.delete(`/api/admin/managed-rooms/${roomId}`, { headers });
    }
  });
});