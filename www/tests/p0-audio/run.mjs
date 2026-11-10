import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const __dirname = fileURLToPath(new URL('.', import.meta.url));
const repoRoot = path.resolve(__dirname, '../../../../..'); // www/tests/p0-audio -> ms_rtc
const evidencePath =
  process.env.P0_EVIDENCE ??
  path.join(repoRoot, 'docs/plans/talkservo-port/evidence/p0-browser-audio.json');
const chromePath =
  process.env.CHROME_PATH ??
  // pick the newest downloaded Playwright chromium (the build number moves per Playwright version)
  (() => {
    const root = path.join(os.homedir(), '.cache/ms-playwright');
    if (!existsSync(root)) return '';
    const candidates = readdirSync(root)
      .filter((d) => /^chromium-\d+$/.test(d))
      .sort()
      .reverse();
    for (const d of candidates) {
      for (const rel of ['chrome-linux64/chrome', 'chrome-linux/chrome']) {
        const p = path.join(root, d, rel);
        if (existsSync(p)) return p;
      }
    }
    return '';
  })();
const APP = 'http://127.0.0.1:5174';
const SERVER = 'http://127.0.0.1:9800';
const ROOM = process.env.P0_ROOM ?? 'audio-p0';

function log(...args) {
  console.log('[p0-audio]', ...args);
}

async function waitFor(url, timeoutMs = 30000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok || res.status < 500) return true;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

async function main() {
  if (!existsSync(chromePath)) {
    throw new Error(
      `chromium not found at ${chromePath}; set CHROME_PATH or install Playwright browsers`,
    );
  }
  if (!(await waitFor(`${SERVER}/healthz`, 3000))) {
    throw new Error(
      `server not healthy at ${SERVER}/healthz; start it first: cd 3rdparty/MediaServo && bash mediaservo.sh up --env dev server`,
    );
  }

  const build = spawnSync('pnpm', ['exec', 'vite', 'build'], {
    cwd: __dirname,
    stdio: 'inherit',
  });
  if (build.status !== 0) throw new Error('vite build failed');

  const vite = spawn('pnpm', ['exec', 'vite', 'preview', '--port', '5174', '--strictPort'], {
    cwd: __dirname,
    stdio: 'inherit',
    detached: true,
  });
  const stopVite = () => {
    try {
      process.kill(-vite.pid, 'SIGTERM');
    } catch {
      // already gone
    }
  };

  let browser;
  const evidence = {
    date: new Date().toISOString(),
    room: ROOM,
    server: SERVER,
    publisher: null,
    consumer: null,
    pass: false,
    error: null,
  };

  try {
    if (!(await waitFor(APP, 30000))) throw new Error(`vite not ready at ${APP}`);

    browser = await chromium.launch({
      executablePath: chromePath,
      headless: true,
      args: [
        '--use-fake-device-for-media-stream',
        '--use-fake-ui-for-media-stream',
        '--autoplay-policy=no-user-gesture-required',
        '--no-proxy-server',
        '--no-sandbox',
      ],
    });

    const pubCtx = await browser.newContext({ permissions: ['microphone'] });
    const pub = await pubCtx.newPage();
    pub.on('console', (m) => log('pub', m.type(), m.text()));
    await pub.goto(`${APP}/?role=pub&room=${ROOM}&user=dispatcher&pass=dispatch123`);
    await pub.waitForFunction(() => window.__p0?.produced === true, { timeout: 40000 });
    evidence.publisher = await pub.evaluate(() => window.__p0);
    log('publisher produced');

    const subCtx = await browser.newContext({ permissions: ['microphone'] });
    const sub = await subCtx.newPage();
    sub.on('console', (m) => log('sub', m.type(), m.text()));
    await sub.goto(`${APP}/?role=sub&room=${ROOM}&user=admin&pass=admin123`);
    await sub.waitForFunction(
      () =>
        window.__p0?.track === true ||
        window.__p0?.status === 'playing' ||
        (window.__p0?.metrics?.bitrate ?? 0) > 0,
      { timeout: 40000 },
    );
    evidence.consumer = await sub.evaluate(() => window.__p0);

    // O24: real RTP flow — byte counters must grow in BOTH directions
    // (PIT-105 lesson: "wired" is not "flowing"; only byte counts prove it).
    const bytesStart = Date.now();
    let pubStats = [];
    let subStats = [];
    while (Date.now() - bytesStart < 15000) {
      pubStats = (await pub.evaluate(() => window.__p0Stats?.() ?? [])) ?? [];
      subStats = (await sub.evaluate(() => window.__p0Stats?.() ?? [])) ?? [];
      const outB = pubStats.reduce((a, s) => a + (s.bytesSent ?? 0), 0);
      const inB = subStats.reduce((a, s) => a + (s.bytesReceived ?? 0), 0);
      if (outB > 0 && inB > 0) break;
      await new Promise((r) => setTimeout(r, 500));
    }
    evidence.publisher.stats = pubStats;
    evidence.consumer.stats = subStats;
    evidence.bytes = {
      publisher_bytes_sent: pubStats.reduce((a, s) => a + (s.bytesSent ?? 0), 0),
      publisher_packets_sent: pubStats.reduce((a, s) => a + (s.packetsSent ?? 0), 0),
      consumer_bytes_received: subStats.reduce((a, s) => a + (s.bytesReceived ?? 0), 0),
      consumer_packets_received: subStats.reduce((a, s) => a + (s.packetsReceived ?? 0), 0),
    };

    evidence.pass =
      evidence.publisher?.produced === true &&
      (evidence.bytes?.publisher_bytes_sent ?? 0) > 0 &&
      (evidence.bytes?.consumer_bytes_received ?? 0) > 0;
    log('consumer state', JSON.stringify(evidence.consumer));
  } catch (e) {
    evidence.error = String(e?.message ?? e);
    log('FAIL', evidence.error);
  } finally {
    if (browser) await browser.close().catch(() => {});
    stopVite();
    mkdirSync(path.dirname(evidencePath), { recursive: true });
    writeFileSync(evidencePath, JSON.stringify(evidence, null, 2) + '\n');
    log('evidence', evidencePath, 'pass=', evidence.pass);
  }

  process.exit(evidence.pass ? 0 : 1);
}

main().catch((e) => {
  console.error('[p0-audio] fatal', e);
  process.exitCode = 1;
});
