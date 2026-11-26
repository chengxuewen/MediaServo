#!/usr/bin/env node
// msrv-shots.mjs — evidence screenshots of the DEPLOYED monitor page, used by
// scripts/msrv-deploy-verify.sh after every stop/build/start cycle.
// Captures: full page, stage bar, left rail, and the SELECTED tile (after clicking a
// rail card) so the selection sync is visually verifiable.
// Run: node www/scripts/msrv-shots.mjs [--app http://10.144.0.18:8080/admin] [--out DIR]
import { createRequire } from 'node:module';
import path from 'node:path';
import fs from 'node:fs';

const SUB = '/home/maxsense/Documents/ms_rtc/3rdparty/MediaServo';
const monReq = createRequire(path.join(SUB, 'www/apps/monitor/package.json'));
const { chromium } = monReq('@playwright/test');
const arg = (n, d) => (process.argv.includes(n) ? process.argv[process.argv.indexOf(n) + 1] : d);
const APP = arg('--app', 'http://10.144.0.18:8080/admin');
const OUT = arg('--out', '/home/maxsense/Documents/ms_rtc/docs/plans/monitor-nvr-visual-parity/evidence/auto');
fs.mkdirSync(OUT, { recursive: true });

let token = '';
try {
  const s = JSON.parse(fs.readFileSync(path.join(SUB, 'www/apps/monitor/.auth/admin.json'), 'utf8'));
  for (const o of s.origins ?? []) for (const kv of o.localStorage ?? []) if (kv.name === 'mediaservo_admin_token') token = kv.value;
} catch { /* no token */ }

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1600, height: 900 }, deviceScaleFactor: 1 });
await ctx.addInitScript((t) => { try { localStorage.setItem('mediaservo_admin_token', t); localStorage.setItem('ms_demo', '1'); } catch { /* silent */ } }, token);
const page = await ctx.newPage();
await page.goto(`${APP}/monitor?demo=1`, { waitUntil: 'networkidle' });
await page.locator('.wall-grid .video-player.nvr').first().waitFor({ timeout: 20000 });
await page.waitForTimeout(800);

const shot = async (name, target) => {
  const file = path.join(OUT, name);
  if (target) await page.locator(target).first().screenshot({ path: file });
  else await page.screenshot({ path: file });
  console.log('shot', name);
};

await shot('full.png');
await shot('toolbar.png', '.wall-toolbar');
await shot('rail.png', '.stream-rail');
// select the 2nd rail card -> selected card + selected tile ring
await page.locator('.rail-card').nth(1).locator('.rc-name').click();
await page.waitForTimeout(400);
await shot('rail-selected.png', '.stream-rail');
await shot('tile-selected.png', '.wall-cell.selected, .wall-offline.selected');
await page.screenshot({ path: path.join(OUT, 'full-selected.png') });
console.log('shot full-selected.png');
// Select-streams dialog: open it, pick two child streams, shoot the dialog + the Selected pane
const rail = page.locator('.stream-rail');
if (!(await rail.isVisible().catch(() => false))) { await page.locator('[aria-label="Toggle stream rail"]').click(); await page.waitForTimeout(300); }
await page.locator('button.open-picker').click();
await page.waitForTimeout(500);
await page.locator('.picker-dev .pick-caret').first().click();
await page.waitForTimeout(200);
const kids = page.locator('.picker-child');
if (await kids.count() > 1) { await kids.nth(0).click(); await kids.nth(1).click(); }
await page.waitForTimeout(300);
await shot('picker.png', '.picker-modal');
await shot('picker-selected.png', '.picker-chosen');
await page.keyboard.press('Escape');
await browser.close();
