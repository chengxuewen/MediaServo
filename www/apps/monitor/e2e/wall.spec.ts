import { test, expect } from './host-fixture';

// S13雏形 (P3 slice ③a): select a live stream from the device tree → single-stream
// playback reaches readyState≥2 with a real first frame (videoWidth>0). The host
// fixture ensures a producing device room; the tree reads producers from /sfu/stats
// (B5 corrected contract), so a PSK host room is selectable (unlike admin /rooms streams).
test.describe('Monitor wall — single stream (S13雏形)', () => {
  test('select stream → video readyState≥2 + first frame', async ({ page, hostProducer }) => {
    await page.goto('/');

    // tree default-expanded (B5 board default): the producing room's video stream row appears
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });
    const streamBtn = page.locator('.st-stream').first();
    await expect(streamBtn).toBeVisible({ timeout: 15000 });
    await streamBtn.click();

    // shared VideoPlayer (packages/react) mounts a modal <video>; assert real media arrival
    const video = page.locator('.video-player video');
    await expect(video).toBeVisible({ timeout: 15000 });
    await expect
      .poll(async () => video.evaluate((v) => (v as HTMLVideoElement).readyState), { timeout: 30000, intervals: [500] })
      .toBeGreaterThanOrEqual(2);
    const dims = await video.evaluate((v) => {
      const el = v as HTMLVideoElement;
      return { w: el.videoWidth, h: el.videoHeight };
    });
    expect(dims.w).toBeGreaterThan(0);
    expect(dims.h).toBeGreaterThan(0);
  });
});