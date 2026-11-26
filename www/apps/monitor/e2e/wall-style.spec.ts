import { test, expect, type Page } from '@playwright/test';

// V34: wall/empty-slot parity with portal-unified.html (light theme tokens):
//   .tile-empty { border: 1px dashed var(--bd2)=#B9C1CE; border-radius: var(--r6)=6px;
//                 gap: var(--sp6)=6px; color: var(--tx3)=#646E82; font-size: var(--fs11)=11px;
//                 background: var(--sunken)=#E9EDF3 }
//   markup: <svg class="ic"><use href="#i-grid"/></svg><span>Empty slot</span>
//   .tiles { gap: var(--sp12)=12px }
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);

test.describe('wall empty-slot parity', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto('/?demo=1');
    // demo has 6 streams; pick the 9 grid so empty slots exist
    await page.locator('.layout-menu').click();
    await page.getByRole('option', { name: '9', exact: true }).click();
    await expect(page.locator('.tile-empty').first()).toBeVisible();
  });

  test('empty slot: dashed strong border, sunken fill, r6, 11px tertiary text', async ({ page }) => {
    const slot = page.locator('.tile-empty').first();
    await expect(slot).toBeVisible();
    expect(await style(page, '.tile-empty', 'background-color')).toBe('rgb(233, 237, 243)');
    expect(await style(page, '.tile-empty', 'border-style')).toBe('dashed');
    expect(await style(page, '.tile-empty', 'border-width')).toBe('1px');
    expect(await style(page, '.tile-empty', 'border-color')).toBe('rgb(185, 193, 206)');
    expect(await style(page, '.tile-empty', 'border-radius')).toBe('6px');
    expect(await style(page, '.tile-empty', 'color')).toBe('rgb(100, 110, 130)');
    expect(await style(page, '.tile-empty', 'font-size')).toBe('11px');
    expect(await style(page, '.tile-empty', 'gap')).toBe('6px');
  });

  test('empty slot shows the 2x2 grid glyph above the label', async ({ page }) => {
    const slot = page.locator('.tile-empty').first();
    await expect(slot).toHaveText('Empty slot');
    const ic = slot.locator('svg.te-ic');
    await expect(ic).toBeVisible();
    const b = (await ic.boundingBox())!;
    expect(Math.round(b.width)).toBe(14);
    expect(Math.round(b.height)).toBe(14);
    // V37: prototype #i-grid is stroked, four 7px squares on a 24 grid
    expect(await style(page, '.te-ic', 'fill')).toBe('none');
    expect(await style(page, '.te-ic', 'stroke-width')).toBe('1.8px');
    expect(await ic.locator('rect').count()).toBe(4);
  });

  test('wall grid gap is 12px (prototype .tiles)', async ({ page }) => {
    expect(await style(page, '.wall-grid', 'gap')).toBe('12px');
  });

  test('all empty slots are uniform height in the 9 grid', async ({ page }) => {
    const heights = await page.evaluate(() =>
      [...document.querySelectorAll('.wall-grid .tile-empty')].map((e) => Math.round(e.getBoundingClientRect().height)));
    expect(heights.length).toBeGreaterThan(2);
    expect(Math.max(...heights) - Math.min(...heights)).toBeLessThanOrEqual(2);
  });
});