import { test, expect, type Page } from '@playwright/test';

// V32: portal topbar parity. Reference measured live from portal-unified.html #topbar:
//   bar h54, padding 0 16, white bg, border-bottom 1px #E4E8EF
//   bell .iconbtn 30x30 r6 (muted fill)
//   status .chip-btn 30 high r6 (green dot + mode text)
//   user .userchip-btn 30 high r6 with .avatar 26..30 square r6 + name + caret
const box = async (page: Page, sel: string) => {
  const b = await page.locator(sel).first().boundingBox();
  expect(b, `${sel} exists`).toBeTruthy();
  return b!;
};
const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);

test.describe('portal topbar', () => {
  test('bar is light with a bottom border and 54px tall', async ({ page }) => {
    await page.goto('/');
    const bar = await box(page, '.header');
    expect(Math.round(bar.height)).toBe(54);
    expect(await style(page, '.header', 'padding')).toBe('0px 16px');
    expect(await style(page, '.header', 'border-bottom-width')).toBe('1px');
    // light surface, dark ink (prototype), not the old dark bar
    const bg = await style(page, '.header', 'background-color');
    expect(bg).toBe('rgb(255, 255, 255)');
    const fg = await style(page, '.header .logo', 'color');
    expect(fg).toBe('rgb(20, 24, 33)');
  });

  test('bell / status / user chips match prototype metrics', async ({ page }) => {
    await page.goto('/');
    const bell = await box(page, '.header-bell');
    expect(Math.round(bell.width)).toBe(30);
    expect(Math.round(bell.height)).toBe(30);
    expect(await style(page, '.header-bell', 'border-radius')).toBe('6px');

    const status = await box(page, '.header-status');
    expect(Math.round(status.height)).toBe(30);
    expect(await style(page, '.header-status', 'border-radius')).toBe('6px');
    await expect(page.locator('.header-status .st-dot')).toBeVisible();
    await expect(page.locator('.header-status b')).toHaveText(/^[A-Z]{2}$/);

    const chip = await box(page, '.user-chip');
    expect(Math.round(chip.height)).toBe(30);
    expect(await style(page, '.user-chip', 'border-radius')).toBe('6px');
    const av = await box(page, '.user-chip .avatar');
    expect(Math.round(av.width)).toBe(26);
    expect(await style(page, '.user-chip .avatar', 'border-radius')).toBe('6px');
  });

  test('user menu opens with role + Sign out, closes on Escape', async ({ page }) => {
    await page.goto('/');
    await page.locator('[aria-label="User menu"]').click();
    await expect(page.locator('.user-menu')).toBeVisible();
    await expect(page.locator('.um-head')).toContainText('admin');
    await expect(page.locator('.logout-btn')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.user-menu')).toHaveCount(0);
  });
});