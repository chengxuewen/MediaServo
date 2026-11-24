import { test, expect } from './host-fixture';

// S13 (P3 slice ③b): multi-stream wall mechanics. The host fixture ensures a
// producing device room (PSK host = single producer); the tree reads producers
// from /sfu/stats (B5). Play All / Watch Selected feed a tile grid (shared
// VideoPlayer tile variant); Preview opens a modal window. cap16 hint + Play All
// >8 confirm need >16 / >8 producers (not exercised by the single-host fixture).
// Serial: the host fixture spawns/reuses one host at a time (no concurrent spawn).
test.describe.serial('Monitor wall (S13, slice ③b)', () => {
  test('Play All → wall tile readyState≥2 + first frame + columns control', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    await page.locator('.st-play-all').first().click();

    const video = page.locator('.wall-grid .video-player video').first();
    await expect(video).toBeVisible({ timeout: 15000 });
    await expect
      .poll(async () => video.evaluate((v) => (v as HTMLVideoElement).readyState), { timeout: 30000, intervals: [500] })
      .toBeGreaterThanOrEqual(2);
    const w = await video.evaluate((v) => (v as HTMLVideoElement).videoWidth);
    expect(w).toBeGreaterThan(0);
    await expect(page.locator('.wt-btn')).toHaveCount(4);
  });

  test('Preview opens a modal video reaching readyState≥2', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    await page.locator('.st-preview').first().click();

    const video = page.locator('.video-player-overlay:not(.tile) video');
    await expect(video).toBeVisible({ timeout: 15000 });
    await expect
      .poll(async () => video.evaluate((v) => (v as HTMLVideoElement).readyState), { timeout: 30000, intervals: [500] })
      .toBeGreaterThanOrEqual(2);
  });

  test('select stream → Watch Selected plays the tile', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    await page.locator('.st-check input').first().check();
    await expect(page.locator('.st-play-all').first()).toContainText('Watch Selected');
    await page.locator('.st-play-all').first().click();

    const video = page.locator('.wall-grid .video-player video').first();
    await expect(video).toBeVisible({ timeout: 15000 });
    await expect
      .poll(async () => video.evaluate((v) => (v as HTMLVideoElement).readyState), { timeout: 30000, intervals: [500] })
      .toBeGreaterThanOrEqual(2);
  });

  test('close a tile removes it; Play All restores (wall lifecycle)', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    await page.locator('.st-play-all').first().click();
    const tile = page.locator('.wall-grid .video-player');
    await expect(tile).toHaveCount(1, { timeout: 15000 });

    await page.locator('.wall-grid .vp-close').first().click();
    await expect(tile).toHaveCount(0);

    await page.locator('.st-play-all').first().click();
    await expect(tile).toHaveCount(1, { timeout: 15000 });
    const video = page.locator('.wall-grid .video-player video').first();
    await expect
      .poll(async () => video.evaluate((v) => (v as HTMLVideoElement).readyState), { timeout: 30000, intervals: [500] })
      .toBeGreaterThanOrEqual(2);
  });
});