// O26 room-model acceptance matrix (P3-5): three browser contexts over the
// real server (talk-o26), fake media. Ordered scenarios, one evidence JSON.
// Hold semantics: explicit pointerdown / pointerup dispatch (a plain click
// would fire down+up instantly and defeat hold-to-talk). Denied reason anchor:
// muted -> 'unauthorized' (Rule 2, state.rs).
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const __dirname = fileURLToPath(new URL('.', import.meta.url));
const appRoot = path.resolve(__dirname, '../../apps/voice');
const evidencePath =
  process.env.O26_EVIDENCE ??
  path.resolve(__dirname, '../../../../../docs/plans/voice-web-app/evidence/o26-matrix.json');
const chromePath = process.env.CHROME_PATH ?? (() => {
  const root = path.join(os.homedir(), '.cache/ms-playwright');
  if (!existsSync(root)) return '';
  for (const d of readdirSync(root).filter((x) => /^chromium-\d+$/.test(x)).sort().reverse()) {
    for (const rel of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
      const p = path.join(root, d, rel);
      if (existsSync(p)) return p;
    }
  }
  return '';
})();
const APP = 'http://127.0.0.1:5175';
const ROOM = 'talk-o26';

const results = [];
const log = (...a) => console.log('[o26]', ...a);

async function waitForText(page, text, timeoutMs = 15000) {
  await page.waitForFunction((t) => document.body.innerText.includes(t), text, { timeout: timeoutMs });
}

async function newContext(browser) {
  return browser.newContext({ viewport: { width: 900, height: 700 } });
}

async function openPage(ctx, pathName, user, pass) {
  const page = await ctx.newPage();
  await page.goto(`${APP}${pathName}`);
  await page.getByPlaceholder('user').fill(user);
  await page.getByPlaceholder('password').fill(pass);
  await page.getByRole('button', { name: 'Enter' }).click();
  return page;
}

/** hold-to-talk primitives: explicit pointerdown / pointerup on the TALK button */
const talkBtn = (page) => page.locator('button', { hasText: 'TALK' });
const hold = async (page) => talkBtn(page).dispatchEvent('pointerdown');
const release = async (page) => talkBtn(page).dispatchEvent('pointerup');

async function scenario(name, fn) {
  try {
    await fn();
    results.push({ scenario: name, pass: true });
    log(`PASS ${name}`);
  } catch (err) {
    let body = '';
    try { body = (await dispatchPageRef.current.locator('body').innerText()).slice(0, 200); } catch { /* noop */ }
    results.push({ scenario: name, pass: false, error: String(err).slice(0, 300), body });
    log(`FAIL ${name}: ${String(err).slice(0, 160)} | dispatch=${JSON.stringify(body)}`);
  }
}
const dispatchPageRef = { current: null };

async function waitQueueChange(page, before) {
  await page.waitForFunction(
    (prev) => {
      const m = document.body.innerText.match(/Queue:\s*([^\n]*)/);
      return m && m[1] !== prev;
    }, before, { timeout: 15000 },
  );
}

const main = async () => {
  mkdirSync(path.dirname(evidencePath), { recursive: true });

  const build = spawnSync('pnpm', ['exec', 'vite', 'build'], { cwd: appRoot, stdio: 'pipe' });
  if (build.status !== 0) throw new Error('voice vite build failed');
  const vite = spawn('pnpm', ['exec', 'vite', 'preview', '--port', '5175', '--strictPort'], {
    cwd: appRoot, stdio: 'ignore', detached: true,
  });
  const cleanupVite = () => { try { process.kill(-vite.pid, 'SIGTERM'); } catch { /* noop */ } };
  process.on('exit', cleanupVite);

  const started = Date.now();
  while (Date.now() - started < 30000) {
    try { const r = await fetch(APP); if (r.ok) break; } catch { /* retry */ }
    await new Promise((r) => setTimeout(r, 500));
  }

  const browser = await chromium.launch({
    executablePath: chromePath,
    headless: true,
    args: [
      '--use-fake-device-for-media-stream',
      '--use-fake-ui-for-media-stream',
      '--autoplay-policy=no-user-gesture-required',
    ],
  });
  try {
    const dispatchCtx = await newContext(browser);
    const f1Ctx = await newContext(browser);
    const f2Ctx = await newContext(browser);
    const dispatch = await openPage(dispatchCtx, `/dispatch/${ROOM}`, 'admin', 'admin123');
    dispatchPageRef.current = dispatch;
    const field1 = await openPage(f1Ctx, `/field/${ROOM}`, 'operator', 'operator123');
    const field2 = await openPage(f2Ctx, `/field/${ROOM}`, 'operator', 'operator123');

    await waitForText(dispatch, 'Floor panel');
    await waitForText(field1, 'TALK');
    await waitForText(field2, 'TALK');
    results.push({ scenario: 'B0 setup: 3 contexts live', pass: true });
    log('setup live');

    await scenario('B-1 re-entry reset (mode exclusive + unmute leftovers)', async () => {
      // the room persists across runs (server in-memory): force a clean base.
      // open mode never queues (capacity unbounded) and stale mutes deny.
      await dispatch.locator('button', { hasText: /^exclusive$/ }).click();
      await waitForText(dispatch, 'Mode: exclusive');
      for (let i = 0; i < 6; i++) {
        const unmute = dispatch.locator('button', { hasText: /^unmute / });
        if (!(await unmute.count())) break;
        await unmute.first().click();
        await new Promise((r) => setTimeout(r, 800));
      }
    });

    await scenario('B1 first hold grants (exclusive single holder)', async () => {
      await hold(field1);
      await waitForText(field1, 'You have the floor');
    });

    await scenario('B2 second hold queues', async () => {
      await hold(field2);
      await waitForText(field2, 'Queued — waiting for the floor');
    });

    await scenario('B3 dispatcher sees grant + queue strip (via resync)', async () => {
      // P2 scoping: queue changes ride private updates + snapshots, not room
      // broadcasts -- the dispatcher resyncs (reload = reconnect + resync).
      await dispatch.reload();
      await waitForText(dispatch, 'Floor panel');
      await waitForText(dispatch, 'Queue:');
      const text = await dispatch.locator('body').innerText();
      if (text.includes('Floor: idle')) throw new Error('floor should not be idle while held');
    });

    await scenario('B4 release promotes the queue head', async () => {
      await release(field1);
      await waitForText(field2, 'You have the floor');
    });

    await scenario('B5 idle after the last release', async () => {
      await release(field2);
      await waitForText(field2, 'Tap and hold to talk');
    });

    await scenario('B6 mute revokes + denied(unauthorized) on re-request', async () => {
      await hold(field2);
      await waitForText(field2, 'You have the floor');
      await dispatch.locator('button', { hasText: /^mute / }).first().click();
      await waitForText(field2, 'Tap and hold to talk'); // grant revoked server-side
      await release(field2);
      await hold(field2);
      await waitForText(field2, 'Denied: unauthorized');
      const unmute = dispatch.locator('button', { hasText: /^unmute / });
      if (await unmute.count()) await unmute.click();
      await release(field2);
      await hold(field2); // fresh request after unmute restores the privilege (A3)
      await waitForText(field2, 'You have the floor');
      await release(field2);
      await waitForText(field2, 'Tap and hold to talk');
    });

    await scenario('B7 mode_set broadcasts (dispatch panel)', async () => {
      await dispatch.locator('button', { hasText: /^open$/ }).click();
      await waitForText(dispatch, 'Mode: open');
      await dispatch.locator('button', { hasText: /^exclusive$/ }).click();
      await waitForText(dispatch, 'Mode: exclusive');
    });

    await scenario('B8 set_priority reorders the queue (+prio)', async () => {
      await hold(field1);
      await waitForText(field1, 'You have the floor');
      await hold(field2);
      await waitForText(field2, 'Queued — waiting for the floor');
      await dispatch.reload();
      await waitForText(dispatch, 'Queue:');
      const before = (await dispatch.locator('body').innerText()).match(/Queue:\s*([^\n]*)/)[1];
      await dispatch.locator('button', { hasText: '+prio' }).first().click();
      await waitQueueChange(dispatch, before);
      await release(field1);
      // promoted by priority, the bumped peer now takes the floor
      await waitForText(field2, 'You have the floor');
      await release(field2);
      await waitForText(field2, 'Tap and hold to talk');
    });

    await scenario('B9 resync after reload (snapshot authority)', async () => {
      await field1.reload();
      await waitForText(field1, 'TALK');
      await waitForText(dispatch, 'Mode: exclusive');
    });

    for (const c of [dispatchCtx, f1Ctx, f2Ctx]) await c.close();
  } finally {
    await browser.close();
  }

  const pass = results.every((r) => r.pass);
  writeFileSync(evidencePath, JSON.stringify({
    generated_at: new Date().toISOString(), room: ROOM, matrix: results, pass,
  }, null, 2));
  log(`evidence ${evidencePath} pass=${pass}`);
  if (!pass) process.exitCode = 1;
};

main().catch((e) => { console.error('[o26] fatal', e); process.exitCode = 1; });
