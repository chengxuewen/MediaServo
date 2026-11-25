import { test, expect } from '@playwright/test';
import { inkBox, inkBoxes } from '../../../pixels/ink';

// V23: pixel guards for the wall console — they measure RENDERED INK, so baseline
// offsets caused by mixed font sizes are caught (DOM box centres hide them).
// Demo mode is used for deterministic content and is clearly labelled.
test.describe('pixel alignment (rendered ink)', () => {
  test('stage-bar controls are vertically centred in the bar', async ({ page }) => {
    await page.goto('/?demo=1');
    const bar = page.locator('.wall-toolbar');
    await expect(bar).toBeVisible();
    const box = (await bar.boundingBox())!;
    const ink = await inkBox(bar); // same coordinate space as the element shot
    expect(Math.abs(ink.cy - box.height / 2)).toBeLessThanOrEqual(2);
  });

  test('left/right rail titles share a baseline (one screenshot, two bands)', async ({ page }) => {
    await page.goto('/?demo=1');
    const panel = page.locator('.monitor-three-panel');
    await expect(page.locator('.stream-rail .rail-title')).toBeVisible();
    const panelBox = (await panel.boundingBox())!;
    const leftBox = (await page.locator('.stream-rail .rail-title').boundingBox())!;
    const rightBox = (await page.locator('.mon-alerts .rail-title').boundingBox())!;
    const [left, right] = await inkBoxes(panel, [
      [leftBox.x - panelBox.x, leftBox.x - panelBox.x + leftBox.width],
      [rightBox.x - panelBox.x, rightBox.x - panelBox.x + rightBox.width],
    ]);
    expect(left.bottom).toBeGreaterThan(0);
    expect(Math.abs(left.bottom - right.bottom)).toBeLessThanOrEqual(1);
  });

  test('toolbar first control aligns with the grid left edge (one screenshot)', async ({ page }) => {
    await page.goto('/?demo=1');
    const canvas = page.locator('.canvas');
    const grid = page.locator('.wall-grid');
    await expect(grid.locator('.wall-cell').first()).toBeVisible();
    const canvasBox = (await canvas.boundingBox())!;
    const gridBox = (await grid.boundingBox())!;
    const togglesBox = (await page.locator('.wall-toolbar .region-toggles').boundingBox())!;
    const [toggles] = await inkBoxes(canvas, [
      [togglesBox.x - canvasBox.x, togglesBox.x - canvasBox.x + 1], // first control's left edge column
    ]);
    expect(Math.abs(toggles.left - (gridBox.x - canvasBox.x))).toBeLessThanOrEqual(2);
  });
});