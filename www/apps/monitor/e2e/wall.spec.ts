import { test, expect } from './host-fixture';

// S13 (P3 slice ③b): multi-stream wall mechanics. The host fixture ensures a
// producing device room (PSK host = single producer); the tree reads producers
// from /sfu/stats (B5). Play All / Watch Selected feed a tile grid (shared
// VideoPlayer tile variant); Preview opens a modal window. cap16 hint + Play All
// >8 confirm need >16 / >8 producers (not exercised by the single-host fixture).
// Serial: the host fixture spawns/reuses one host at a time (no concurrent spawn).
test.describe.serial('Monitor wall (S13, slice ③b)', () => {
  test('Play All → wall tile readyState≥2 + first frame + layout control', async ({ page, hostProducer }) => {
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
    await expect(page.locator('.layout-menu')).toBeVisible();
  });

  test('Preview opens a modal video reaching readyState≥2', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });

    await page.locator('.st-preview').first().click();

    const video = page.locator('.vp-modal-wide video');
    await expect(video).toBeVisible({ timeout: 15000 });
    // H3: the expand window is now a real dialog (kit Modal — role/Esc/trap).
    await expect(page.locator('.vp-modal-wide[role="dialog"]')).toBeVisible();
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

  test('B2 named-layout switch changes the wall grid columns', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });

    const colCount = () => page.locator('.wall-grid').evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length);
    await page.locator('.layout-menu').selectOption('1');
    expect(await colCount()).toBe(1);
    await page.locator('.layout-menu').selectOption('4');
    expect(await colCount()).toBe(2);
    await page.locator('.layout-menu').selectOption('9');
    expect(await colCount()).toBe(3);
  });

  test('B2 single-tile zoom fills the wall; exit restores (K5)', async ({ page, hostProducer }) => {
    await page.goto('/');
    await expect(page.locator('.st-device-name').first()).toContainText(hostProducer.room, { timeout: 15000 });
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });

    await page.locator('.wall-grid .video-player').first().hover();
    await page.locator('.tile-actions [aria-label="Zoom this stream"]').first().click();
    await expect(page.locator('.wt-exit-zoom')).toBeVisible();
    const cols = await page.locator('.wall-grid').evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length);
    expect(cols).toBe(1);
    await page.locator('.wt-exit-zoom').click();
    await expect(page.locator('.wt-exit-zoom')).toHaveCount(0);
  });
});