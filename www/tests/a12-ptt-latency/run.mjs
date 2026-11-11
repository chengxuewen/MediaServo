// A12 driver: pub + sub contexts over the real server, one warmup cycle, then
// N measured hold cycles. Latency = press t0 (pub page) -> first RMS edge on
// the remote stream (sub page); press->grant is recorded as the breakdown.
// Acceptance (archived PLAN row): p50 <= 350 ms LAN source budget.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const __dirname = fileURLToPath(new URL('.', import.meta.url));
const evidencePath =
  process.env.A12_EVIDENCE ??
  // 5 levels up = ms_rtc root (www/tests/a12-ptt-latency -> ...)
  path.resolve(__dirname, '../../../../../docs/plans/voice-web-app/evidence/a12-ptt-latency.json');
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
const APP = 'http://127.0.0.1:5176';
const SERVER = 'http://127.0.0.1:9800';
const ROOM = process.env.A12_ROOM ?? 'talk-a12';
const CYCLES = Number(process.env.A12_CYCLES ?? 25);
const BUDGET_MS = 350; // p50 acceptance: archived PLAN "LAN source budget"

const log = (...a) => console.log('[a12]', ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const round1 = (v) => (typeof v === 'number' ? Math.round(v * 10) / 10 : v);

function statsOf(values) {
  const s = [...values].sort((a, b) => a - b);
  const at = (p) => (s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : null);
  return {
    n: s.length,
    min_ms: round1(s[0] ?? null),
    p50_ms: round1(at(0.5)),
    p95_ms: round1(at(0.95)),
    max_ms: round1(s[s.length - 1] ?? null),
    mean_ms: round1(s.length ? s.reduce((a, b) => a + b, 0) / s.length : null),
  };
}

async function main() {
  mkdirSync(path.dirname(evidencePath), { recursive: true });

  const health = await fetch(`${SERVER}/healthz`).then((r) => r.status).catch(() => 0);
  if (health === 0) throw new Error(`server not reachable at ${SERVER}/healthz`);

  const build = spawnSync('pnpm', ['exec', 'vite', 'build'], { cwd: __dirname, stdio: 'pipe' });
  if (build.status !== 0) {
    throw new Error(`vite build failed: ${build.stderr?.toString().slice(0, 400)}`);
  }
  const vite = spawn('pnpm', ['exec', 'vite', 'preview', '--port', '5176', '--strictPort'], {
    cwd: __dirname, stdio: 'ignore', detached: true,
  });
  const cleanupVite = () => { try { process.kill(-vite.pid, 'SIGTERM'); } catch { /* noop */ } };
  process.on('exit', cleanupVite);

  const started = Date.now();
  while (Date.now() - started < 30000) {
    try { const r = await fetch(APP); if (r.ok) break; } catch { /* retry */ }
    await sleep(500);
  }

  const browser = await chromium.launch({
    executablePath: chromePath,
    headless: true,
    // no fake-media flags: the publisher synthesizes its tone, nothing calls
    // getUserMedia; autoplay flag only unlocks the AudioContexts.
    args: ['--autoplay-policy=no-user-gesture-required', '--no-proxy-server', '--no-sandbox'],
  });

  const evidence = {
    date: new Date().toISOString(),
    room: ROOM,
    server: SERVER,
    metric: 'hold-to-talk press -> first RMS edge at listener (LAN source budget)',
    method: 'continuous 440Hz oscillator producer; R6 pause = zero-RTP silence; AnalyserNode edge',
    budget_ms: BUDGET_MS,
    cycles: CYCLES,
    samples: [],
    pass: false,
    error: null,
  };

  try {
    const pubCtx = await browser.newContext();
    const subCtx = await browser.newContext();
    const pub = await pubCtx.newPage();
    const sub = await subCtx.newPage();
    pub.on('console', (m) => { if (m.type() === 'error') log('pub', m.text().slice(0, 160)); });
    sub.on('console', (m) => { if (m.type() === 'error') log('sub', m.text().slice(0, 160)); });

    await pub.goto(`${APP}/?role=pub&room=${ROOM}&user=operator&pass=operator123`);
    await pub.waitForFunction(() => window.__a12?.produced === true || window.__a12?.error, null, { timeout: 40000 });
    const pubErr = await pub.evaluate(() => window.__a12.error);
    if (pubErr) throw new Error(`pub setup failed: ${pubErr}`);
    log('publisher produced');

    await sub.goto(`${APP}/?role=sub&room=${ROOM}&user=admin&pass=admin123`);
    await sub.waitForFunction(() => window.__a12?.track === true || window.__a12?.error, null, { timeout: 40000 });
    const subErr = await sub.evaluate(() => window.__a12.error);
    if (subErr) throw new Error(`sub setup failed: ${subErr}`);
    await sleep(800); // consumer + meter settle
    log('listener track live');

    const cycle = async (label) => {
      await sub.evaluate(() => window.__a12Arm());
      await sub.waitForFunction(() => (window.__a12.rms ?? 0) < 0.01, null, { timeout: 5000 });
      await pub.evaluate(() => window.__a12Press());
      await sub.waitForFunction(() => window.__a12.edge != null, null, { timeout: 5000 });
      const [pubS, subS] = await Promise.all([
        pub.evaluate(() => ({ t0: window.__a12.t0, grantedAt: window.__a12.grantedAt })),
        sub.evaluate(() => ({ edge: window.__a12.edge, edgeRms: window.__a12.edgeRms, armRms: window.__a12.armRms })),
      ]);
      const sample = {
        cycle: label,
        press_to_grant_ms: pubS.grantedAt != null ? round1(pubS.grantedAt - pubS.t0) : null,
        press_to_edge_ms: round1(subS.edge - pubS.t0),
        arm_rms: round1(subS.armRms ?? null),
        edge_rms: round1(subS.edgeRms ?? null),
      };
      log(`${label}: press->grant ${sample.press_to_grant_ms}ms, press->edge ${sample.press_to_edge_ms}ms`);
      await pub.evaluate(() => window.__a12Release());
      await pub.waitForFunction(() => window.__a12.granted === false, null, { timeout: 5000 });
      await sub.waitForFunction(() => (window.__a12.rms ?? 0) < 0.01, null, { timeout: 5000 });
      await sleep(200);
      return sample;
    };

    await cycle('warmup'); // first grant/resume warms the resume path; not counted

    for (let i = 1; i <= CYCLES; i++) {
      evidence.samples.push(await cycle(i));
    }

    const edge = statsOf(evidence.samples.map((s) => s.press_to_edge_ms));
    const grant = statsOf(evidence.samples.map((s) => s.press_to_grant_ms).filter((v) => v != null));
    evidence.stats_edge = edge;
    evidence.stats_grant_breakdown = grant;
    evidence.pass = edge.p50_ms != null && edge.p50_ms <= BUDGET_MS;
    log(`p50=${edge.p50_ms}ms p95=${edge.p95_ms}ms budget=${BUDGET_MS}ms pass=${evidence.pass}`);
  } catch (e) {
    evidence.error = String(e?.message ?? e).slice(0, 300);
    log('FAIL', evidence.error);
  } finally {
    await browser.close().catch(() => {});
    cleanupVite();
    writeFileSync(evidencePath, JSON.stringify(evidence, null, 2) + '\n');
    log('evidence', evidencePath, 'pass=', evidence.pass);
  }

  process.exit(evidence.pass ? 0 : 1);
}

main().catch((e) => {
  console.error('[a12] fatal', e);
  process.exitCode = 1;
});
