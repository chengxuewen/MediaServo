#!/usr/bin/env node
// ui-conformance (C2/C3): the MISSING fourth verification layer — design conformance.
// Dual screenshots of the DESIGN AUTHORITY (prototype) vs the IMPLEMENTATION at the
// same viewport, pixel-diffed (pixelmatch), producing a gap ledger + diff image.
// Unlike self-baseline regression (app vs app), this compares against the PROTOTYPE,
// so "looks like the design" is machine-measured, never self-asserted.
//
// Run: node www/scripts/ui-conformance.mjs --page monitor --mode demo|real [--app URL] [--out DIR]
// Exit 0 always (ledger tool); CRITICAL layout diffs are reviewed/fixed per C3, not auto-pass.
import { createRequire } from 'node:module';
import path from 'node:path';
import fs from 'node:fs';

const SUB = '/home/maxsense/Documents/ms_rtc/3rdparty/MediaServo';
const wwwReq = createRequire(path.join(SUB, 'www/package.json'));
const { PNG } = wwwReq('pngjs');
const pm = wwwReq('pixelmatch');
const pixelmatch = pm.default || pm;
const monReq = createRequire(path.join(SUB, 'www/apps/monitor/package.json'));
const { chromium } = monReq('@playwright/test');

const arg = (n, d) => process.argv.includes(n) ? process.argv[process.argv.indexOf(n) + 1] : d;
const page = arg('--page', 'monitor');
const mode = arg('--mode', 'demo');
const APP = arg('--app', 'http://localhost:5176/');
const OUT = arg('--out', '/home/maxsense/Documents/ms_rtc/docs/plans/monitor-nvr-visual-parity/evidence/conformance');
const PROTO = path.join(SUB, 'docs/ui/prototypes/portal-unified.html');
const VP = { width: 1600, height: 900 };
fs.mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch();
try {
  // Design authority (prototype), simulated by design.
  const pp = await browser.newPage({ viewport: VP });
  await pp.goto('file://' + PROTO);
  await pp.waitForSelector('#page-overview.active');
  await pp.evaluate((p) => goto(p), page);
  await pp.waitForTimeout(400);
  const protoBuf = await pp.screenshot();
  await pp.close();

  // Implementation. demo = labeled preview (like-for-like data); real = honest live state.
  const ctx = await browser.newContext({ viewport: VP, storageState: path.join(SUB, 'www/apps/monitor/.auth/admin.json') });
  const ip = await ctx.newPage();
  await ip.goto(APP + (mode === 'demo' ? '?demo=1' : ''));
  await ip.waitForSelector('.monitor-home');
  await ip.waitForTimeout(400);
  const implBuf = await ip.screenshot();
  await ctx.close();

  const a = PNG.sync.read(protoBuf);
  const b = PNG.sync.read(implBuf);
  const w = Math.min(a.width, b.width), h = Math.min(a.height, b.height);
  const crop = (img) => { const o = new PNG({ width: w, height: h }); PNG.bitblt(img, o, 0, 0, w, h, 0, 0); return o; };
  const ca = crop(a), cb = crop(b);
  const diff = new PNG({ width: w, height: h });
  const mismatch = pixelmatch(ca.data, cb.data, diff.data, w, h, { threshold: 0.1 });
  const pct = (100 * mismatch / (w * h)).toFixed(1);

  fs.writeFileSync(path.join(OUT, `proto-${page}.png`), protoBuf);
  fs.writeFileSync(path.join(OUT, `impl-${page}-${mode}.png`), implBuf);
  fs.writeFileSync(path.join(OUT, `diff-${page}-${mode}.png`), PNG.sync.write(diff));
  const report = { page, mode, viewport: VP, mismatchPixels: mismatch, mismatchPct: Number(pct), note: 'pixel diff vs PROTOTYPE (design authority). High % = real gap ledger, NOT a pass/fail of chrome parity; review CRITICAL layout diffs per C3.' };
  fs.writeFileSync(path.join(OUT, `report-${page}-${mode}.json`), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); }