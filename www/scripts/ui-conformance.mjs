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
  // Geometry ledger: bounding boxes of key chrome regions (layout-level, video-agnostic).
  const REG = [
    ['stagebar', '.stage-bar', '.wall-toolbar'],
    ['leftrail', '.stream-rail', '.stream-rail'],
    ['wall', '.tiles', '.wall-grid'],
    ['alerts', '.mon-alerts', '.mon-alerts'],
    ['bottom', '.ctl-bar', '.ctl-bar'],
  ];
  const grab = (page, idx) => page.evaluate(([regs, id]) => regs.map(([k, p, i]) => {
    const el = document.querySelector(id === 0 ? p : i);
    if (!el) return { k, present: false };
    const r = el.getBoundingClientRect();
    return { k, present: true, x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
  }), [REG, idx]);
  const protoRects = await grab(pp, 0);
  await pp.close();

  // Implementation. demo = labeled preview (like-for-like data); real = honest live state.
  // VISUAL_INSHELL=1 compares the REAL product (admin /monitor inside the shell) — same
  // 230px nav context as the prototype — instead of the standalone app.
  const INSHELL = !!process.env.VISUAL_INSHELL;
  let token = '';
  if (INSHELL) {
    try {
      const s = JSON.parse(fs.readFileSync(path.join(SUB, 'www/apps/monitor/.auth/admin.json'), 'utf8'));
      for (const o of s.origins ?? []) for (const kv of o.localStorage ?? []) if (kv.name === 'mediaservo_admin_token') token = kv.value;
    } catch { /* no token */ }
  }
  const ctx = await browser.newContext({ viewport: VP, storageState: INSHELL ? undefined : path.join(SUB, 'www/apps/monitor/.auth/admin.json') });
  if (INSHELL) await ctx.addInitScript((t) => { try { localStorage.setItem('mediaservo_admin_token', t); } catch { /* silent */ } }, token);
  const ip = await ctx.newPage();
  await ip.goto(INSHELL ? `${APP}/monitor${mode === 'demo' ? '?demo=1' : ''}` : APP + (mode === 'demo' ? '?demo=1' : ''));
  await ip.waitForSelector('.monitor-home');
  await ip.waitForTimeout(400);
  const implBuf = await ip.screenshot();
  const implRects = await grab(ip, 1);
  await ctx.close();
  // CRITICAL layout ledger: region present/size deltas (tolerance 8px), video-agnostic.
  const ledger = protoRects.map((p, i) => {
    const m = implRects[i];
    if (!p.present || !m.present) return { region: p.k, critical: true, reason: `present proto=${p.present} impl=${m.present}` };
    const dh = Math.abs(p.h - m.h), dw = Math.abs(p.w - m.w), dx = Math.abs(p.x - m.x);
    // x/width offsets come from the GLOBAL shell (prototype has a 230px portal nav the
    // standalone app lacks) — accounted to admin-restyle, not a monitor-chrome defect.
    const shellOffset = dx > 100;
    return { region: p.k, proto: p, impl: m, heightDelta: dh, widthDelta: dw, shellOffset, critical: !shellOffset && dh > 8 };
  });

  const a = PNG.sync.read(protoBuf);
  const b = PNG.sync.read(implBuf);
  const w = Math.min(a.width, b.width), h = Math.min(a.height, b.height);
  const crop = (img) => { const o = new PNG({ width: w, height: h }); PNG.bitblt(img, o, 0, 0, w, h, 0, 0); return o; };
  const ca = crop(a), cb = crop(b);
  const diff = new PNG({ width: w, height: h });
  const mismatch = pixelmatch(ca.data, cb.data, diff.data, w, h, { threshold: 0.1 });
  const pct = (100 * mismatch / (w * h)).toFixed(1);

  const tag = INSHELL ? 'inshell' : mode;
  fs.writeFileSync(path.join(OUT, `proto-${page}.png`), protoBuf);
  fs.writeFileSync(path.join(OUT, `impl-${page}-${tag}.png`), implBuf);
  fs.writeFileSync(path.join(OUT, `diff-${page}-${tag}.png`), PNG.sync.write(diff));
  const report = { page, mode: tag, viewport: VP, mismatchPixels: mismatch, mismatchPct: Number(pct), layoutLedger: ledger, note: 'pixel % dominated by global shell + video content (data); use layoutLedger (region geometry, video-agnostic) for CRITICAL layout diffs per C3.' };
  fs.writeFileSync(path.join(OUT, `report-${page}-${tag}.json`), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ ...report, layoutLedger: ledger.map((l) => `${l.region}:${l.critical ? 'CRIT' : l.shellOffset ? 'shell' : 'ok'}${l.heightDelta != null ? `(h${l.heightDelta}/w${l.widthDelta})` : ''}`) }));
} finally { await browser.close(); }