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

  test('900px stays 3 docked columns (rails not overlaying the page)', async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 860 });
    await page.goto('/?demo=1');
    const toolbar = page.locator('.wall-toolbar');
    const grid = page.locator('.wall-grid');
    await expect(toolbar).toBeVisible();
    // V28: rails dock inside the monitor area down to 768px (prototype behaviour).
    await expect(page.locator('.stream-rail')).toBeVisible();
    await expect(page.locator('.mon-alerts')).toBeVisible();
    const rail = (await page.locator('.stream-rail').boundingBox())!;
    const alerts = (await page.locator('.mon-alerts').boundingBox())!;
    const tb = (await toolbar.boundingBox())!;
    const gb = (await grid.boundingBox())!;
    // docked columns share the toolbar's top; the wall sits between them
    expect(Math.abs(rail.y - tb.y)).toBeLessThanOrEqual(2);
    expect(Math.abs(alerts.y - tb.y)).toBeLessThanOrEqual(2);
    expect(tb.width).toBeGreaterThan(250);
    expect(gb.width).toBeGreaterThan(200);
    expect(gb.height).toBeGreaterThan(200);
    // rails are in-flow columns, not overlays: the wall starts right of the rail
    expect(Math.round(gb.x)).toBeGreaterThanOrEqual(Math.round(rail.x + rail.width));
  });

  test('below 768px the wall is visible and drawers start closed', async ({ page }) => {
    await page.setViewportSize({ width: 700, height: 860 });
    await page.goto('/?demo=1');
    const grid = page.locator('.wall-grid');
    await expect(page.locator('.wall-toolbar')).toBeVisible();
    await expect(grid).toBeVisible();
    expect((await grid.boundingBox())!.width).toBeGreaterThan(300);
    await expect(page.locator('.stream-rail')).toBeHidden();
    await expect(page.locator('.mon-alerts')).toBeHidden();
  });

  test('stage-bar never clips or wraps when the canvas is narrow', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 800 }); // both rails open -> narrow canvas
    await page.goto('/?demo=1');
    const bar = page.locator('.wall-toolbar');
    await expect(bar).toBeVisible();
    const overflow = await page.evaluate(() => {
      const el = document.querySelector('.wall-toolbar')! as HTMLElement;
      const r = el.getBoundingClientRect();
      const kids = [...el.children].filter((c) => getComputedStyle(c).display !== 'none') as HTMLElement[];
      const worst = Math.max(...kids.map((c) => Math.round(c.getBoundingClientRect().right - r.right)));
      return { worst, scroll: el.scrollWidth - el.clientWidth, h: Math.round(el.getBoundingClientRect().height) };
    });
    expect(overflow.worst).toBeLessThanOrEqual(1); // nothing sticks out of the bar
    expect(overflow.scroll).toBeLessThanOrEqual(1); // no hidden overflow
    // V31: the prototype WRAPS instead of hiding controls (max 2 rows)
    expect(overflow.h).toBeLessThanOrEqual(90);
    await expect(page.locator('.wall-toolbar .health-chip')).toBeVisible();
    await expect(page.locator('.live-clock')).toBeVisible();
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