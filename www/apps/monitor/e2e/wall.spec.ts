import { test, expect } from './host-fixture';
import { readyProducers, playAllFirst } from './flow';

// S13 (P3 slice ③b): multi-stream wall mechanics. V9b (K12=A): the device tree lives in
// the Picker; the rail shows selected cards. Non-prototype extras (per-stream Preview /
// Watch Selected) were retired — the prototype has neither. Serial: one host at a time.
test.describe.serial('Monitor wall (S13, slice ③b)', () => {
  test('Play All → wall tile readyState≥2 + first frame + layout control', async ({ page, hostProducer }) => {
    await page.goto('/');
    await readyProducers(page, hostProducer.room);
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

  test('close a tile removes it; Play All restores (wall lifecycle)', async ({ page, hostProducer }) => {
    await page.goto('/');
    await readyProducers(page, hostProducer.room);
    await page.locator('.st-play-all').first().click();
    const tile = page.locator('.wall-grid .video-player');
    await expect(tile).toHaveCount(1, { timeout: 15000 });

    // V36: removal lives in the tile's hover action cluster
    await page.locator('.wall-grid .video-player').first().hover();
    await page.locator('.wall-grid .tile-del').first().click();
    await expect(tile).toHaveCount(0);

    await playAllFirst(page);
    await expect(tile).toHaveCount(1, { timeout: 15000 });
    const video = page.locator('.wall-grid .video-player video').first();
    await expect
      .poll(async () => video.evaluate((v) => (v as HTMLVideoElement).readyState), { timeout: 30000, intervals: [500] })
      .toBeGreaterThanOrEqual(2);
  });

  test('B2 named-layout switch changes the wall grid columns', async ({ page, hostProducer }) => {
    await page.goto('/');
    await readyProducers(page, hostProducer.room);
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });

    const colCount = () => page.locator('.wall-grid').evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length);
    const pick = async (name: string) => { await page.locator('.layout-menu').click(); await page.getByRole('option', { name, exact: true }).click(); };
    await pick('1');
    expect(await colCount()).toBe(1);
    await pick('4');
    expect(await colCount()).toBe(2);
    await pick('9');
    expect(await colCount()).toBe(3);
  });

  test('V2 layout gallery keyboard nav (arrows + Enter)', async ({ page, hostProducer }) => {
    await page.goto('/');
    await readyProducers(page, hostProducer.room);
    await page.locator('.st-play-all').first().click();
    await expect(page.locator('.wall-grid .video-player')).toHaveCount(1, { timeout: 15000 });
    await page.locator('.layout-menu').click();
    await expect(page.locator('.layout-gallery')).toBeVisible();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Enter');
    await expect(page.locator('.layout-gallery')).toHaveCount(0);
    const cols = await page.locator('.wall-grid').evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length);
    expect(cols).toBe(1);
  });

  test('B2 single-tile zoom fills the wall; exit restores (K5)', async ({ page, hostProducer }) => {
    await page.goto('/');
    await readyProducers(page, hostProducer.room);
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