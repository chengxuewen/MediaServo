import { test, expect, type Page } from '@playwright/test';

// V35: live-tile chrome parity with portal-unified.html (measured):
//   .tile-bar  { position:absolute; padding:6px 8px; gradient; font-size:12px;
//                color:rgba(255,255,255,.84) }
//   .tile-name { font-weight:600; nowrap; text-shadow }
//   .tile-btn  { 22x22; r4; border rgba(255,255,255,.22); bg rgba(0,0,0,.55) }
//   .g         { 12x12 } online dot / warn triangle / offline slash
//   .tile-live { 11px; 600; padding 2px 6px; r2 }
//   .tile.degraded { border:1px dashed warn }
//   .tile-stats{ hidden until hover }
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);
const box = async (page: Page, sel: string) => {
  const b = await page.locator(sel).first().boundingBox();
  expect(b, `${sel} exists`).toBeTruthy();
  return b!;
};

test.describe('live tile chrome', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.wall-grid .video-player.nvr').first()).toBeVisible();
  });

  test('top bar is an absolute gradient overlay that does not push the video down', async ({ page }) => {
    expect(await style(page, '.video-player.nvr .vp-top-bar', 'position')).toBe('absolute');
    expect(await style(page, '.video-player.nvr .vp-top-bar', 'background-image')).toContain('linear-gradient');
    expect(await style(page, '.video-player.nvr .vp-top-bar', 'padding')).toBe('6px 96px 6px 8px'); // room for 3 buttons
    expect(await style(page, '.video-player.nvr .vp-top-bar', 'font-size')).toBe('12px');
    expect(await style(page, '.video-player.nvr .vp-top-bar', 'color')).toBe('rgba(255, 255, 255, 0.84)');
    const tile = await box(page, '.wall-grid .video-player.nvr');
    const body = await box(page, '.wall-grid .video-player.nvr .vp-body');
    expect(Math.abs(body.y - tile.y)).toBeLessThanOrEqual(1);
  });

  test('name is one 12px/600 line with a shadow; close is a hover-revealed 22px square', async ({ page }) => {
    expect(await style(page, '.video-player.nvr .vp-title', 'font-size')).toBe('12px');
    expect(await style(page, '.video-player.nvr .vp-title', 'font-weight')).toBe('600');
    expect(await style(page, '.video-player.nvr .vp-title', 'white-space')).toBe('nowrap');
    expect(await style(page, '.video-player.nvr .vp-title', 'text-shadow')).not.toBe('none');

    // V36: ONE action cluster top-right (2x2 / zoom / remove), hover-revealed
    const cluster = page.locator('.wall-grid .video-player.nvr .tile-actions').first();
    expect(await style(page, '.video-player.nvr .tile-actions', 'opacity')).toBe('0');
    await page.locator('.wall-grid .video-player.nvr').first().hover();
    await expect.poll(async () => style(page, '.video-player.nvr .tile-actions', 'opacity')).toBe('1');
    await expect(cluster.getByRole('button', { name: 'Zoom this stream' })).toBeVisible();
    await expect(cluster.getByRole('button', { name: 'Remove from wall' })).toBeVisible();
    const btn = await box(page, '.video-player.nvr .tile-btn');
    expect(Math.round(btn.width)).toBe(22);
    expect(Math.round(btn.height)).toBe(22);
    expect(await style(page, '.video-player.nvr .tile-btn', 'border-radius')).toBe('4px');
    expect(await style(page, '.video-player.nvr .tile-actions', 'gap')).toBe('6px');
  });

  test('status glyph is 12x12 and green on a playing tile', async ({ page }) => {
    const g = page.locator('.video-player.nvr .tile-glyph.online').first();
    await expect(g).toBeAttached();
    const b = await g.boundingBox();
    expect(Math.round(b!.width)).toBe(12);
    expect(Math.round(b!.height)).toBe(12);
    expect(await style(page, '.video-player.nvr .tile-glyph.online', 'background-color')).toContain('31, 157, 85');
  });

  test('LIVE badge 11px/600 2px-6px r2; degraded tile has a dashed border', async ({ page }) => {
    expect(await style(page, '.video-player.nvr .tile-live.live', 'font-size')).toBe('11px');
    expect(await style(page, '.video-player.nvr .tile-live.live', 'font-weight')).toBe('600');
    expect(await style(page, '.video-player.nvr .tile-live.live', 'padding')).toBe('2px 6px');
    expect(await style(page, '.video-player.nvr .tile-live.live', 'border-radius')).toBe('2px');
    await expect(page.locator('.video-player.nvr.degraded').first()).toBeAttached();
    expect(await style(page, '.video-player.nvr.degraded', 'border-style')).toBe('dashed');
  });

  test('mini stats hidden until the tile is hovered (prototype .tile-stats)', async ({ page }) => {
    expect(await style(page, '.video-player.nvr .vp-mini-stats', 'opacity')).toBe('0');
    await page.locator('.wall-grid .video-player.nvr').first().hover();
    await expect.poll(async () => style(page, '.video-player.nvr .vp-mini-stats', 'opacity')).toBe('1');
  });
});
