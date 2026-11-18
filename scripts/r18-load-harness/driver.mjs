// R18 Small-tier load driver (container-build-rework). Launches N chromium peers
// producing fake 720p H.264 ~2Mbps + consuming, samples `docker stats` every 5s
// across a steady window + a burst window, and computes mem_limit = peak x1.3
// round-up to 512MB. Honest measurement: records achieved per-peer bitrate.
import { chromium } from 'playwright';
import { execSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';

const CFG = {
  psk: process.env.PSK,
  wsUrl: process.env.WS_URL || 'ws://127.0.0.1:19800/ws',
  container: process.env.CONTAINER || 'msrtc-server-1',
  rooms: parseInt(process.env.ROOMS || '10'),
  perRoom: parseInt(process.env.PER_ROOM || '2'),
  steady: parseInt(process.env.STEADY || '600'),
  burst: parseInt(process.env.BURST || '120'),
  bitrate: parseInt(process.env.BITRATE || '2000000'),
  fps: parseInt(process.env.FPS || '30'),
  out: process.env.OUT || 'r18-result.json',
  chrome: process.env.CHROME || '/home/maxsense/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome',
};
if (!CFG.psk) { console.error('PSK env required'); process.exit(2); }
const bundle = readFileSync(new URL('./load.bundle.js', import.meta.url), 'utf8');
const log = (...a) => console.log(`[${new Date().toISOString().slice(11, 19)}]`, ...a);

// Serve the load page from a real HTTP origin (about:blank's null origin makes
// chromium block the WebSocket). The page origin = http://127.0.0.1:8099.
const PAGE_PORT = parseInt(process.env.PAGE_PORT || '8099');
const html = `<!doctype html><html><head><meta charset="utf-8"></head><body><script>${bundle}</script></body></html>`;
const httpSrv = createServer((req, res) => { res.writeHead(200, { 'Content-Type': 'text/html' }); res.end(html); });
await new Promise((r) => httpSrv.listen(PAGE_PORT, '127.0.0.1', r));
const pageUrl = `http://127.0.0.1:${PAGE_PORT}/`;

function memMiB() {
  try {
    const s = execSync(`docker stats ${CFG.container} --no-stream --format '{{.MemUsage}}'`, { encoding: 'utf8' }).trim();
    const m = s.match(/([\d.]+)([KMGT]?i?B)/);
    if (!m) return null;
    const v = parseFloat(m[1]); const u = m[2];
    const mult = { B: 1 / 1048576, KiB: 1 / 1024, MiB: 1, GiB: 1024, TiB: 1048576, KB: 1 / 1048.576, MB: 1 / 1.048576, GB: 953.674 }[u] ?? 1;
    return v * mult;
  } catch { return null; }
}

const samples = [];
let peak = 0;
function sample(phase) {
  const m = memMiB();
  if (m != null) { peak = Math.max(peak, m); samples.push({ t: Date.now(), phase, memMiB: +m.toFixed(2) }); }
  return m;
}

async function newPeerPage(browser, roomId, po) {
  const page = await browser.newPage();
  await page.goto(pageUrl, { waitUntil: 'load' });
  await page.evaluate((o) => window.__runPeer(o), {
    wsUrl: CFG.wsUrl, psk: CFG.psk, roomId, bitrate: CFG.bitrate, fps: CFG.fps,
    role: po.role, produce: po.produce, consume: po.consume, produceDelayMs: po.produceDelayMs || 0,
  }).catch((e) => log('peer runPeer rejected:', e.message));
  return { page, role: po.role, produce: po.produce };
}

async function waitReady(peers, timeoutMs = 90000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    let okCount = 0;
    for (const p of peers) {
      const s = await p.page.evaluate(() => window.__r18stats).catch(() => null);
      if (!s) continue;
      if (p.produce ? s.produced : (s.consumers > 0)) okCount++;
    }
    if (okCount === peers.length) return okCount;
    await new Promise((r) => setTimeout(r, 2000));
  }
  return -1;
}

async function collectStats(peers) {
  let out = 0, inb = 0, produced = 0, consumers = 0, widths = new Set(), errors = [];
  const conns = [];
  for (const p of peers) {
    const s = await p.page.evaluate(() => window.__r18stats).catch(() => null);
    if (!s) continue;
    if (s.produced) produced++;
    out += s.outboundMbps || 0; inb += s.inboundMbps || 0; consumers += s.consumers || 0;
    if (s.videoWidth) widths.add(s.videoWidth);
    errors.push(...(s.errors || []));
    conns.push({ role: p.role, send: s.sendConn, recv: s.recvConn, muted: s.trackMuted, out: s.outboundMbps, in: s.inboundMbps });
  }
  return { produced, consumers, totalOutMbps: +out.toFixed(2), totalInMbps: +inb.toFixed(2), widths: [...widths], conns, errors: errors.slice(0, 10) };
}

const browser = await chromium.launch({
  executablePath: CFG.chrome, headless: true,
  args: ['--headless=new', '--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream',
    `--use-file-for-fake-video-capture=${process.env.FAKE_VIDEO || new URL('./fake.y4m', import.meta.url).pathname}`,
    '--autoplay-policy=no-user-gesture-required', '--disable-features=WebRtcHideLocalIpsWithMdns',
    '--no-sandbox', '--disable-dev-shm-usage'],
});
log(`launch: rooms=${CFG.rooms} perRoom=${CFG.perRoom} bitrate=${CFG.bitrate} fps=${CFG.fps} steady=${CFG.steady}s burst=${CFG.burst}s`);

const peers = [];
// Phase 1: hosts produce first (creates the room Router; remotes need it for caps).
const hosts = [];
for (let r = 0; r < CFG.rooms; r++) {
  const h = await newPeerPage(browser, `load-room-${r}`, { role: 'host', produce: true, consume: false, produceDelayMs: 0 });
  hosts.push(h); peers.push(h);
  await new Promise((res) => setTimeout(res, 200));
}
log(`created ${hosts.length} hosts; waiting produce...`);
await waitReady(hosts, 60000);
await new Promise((r) => setTimeout(r, 3000)); // let producers settle + router ready
// Phase 2: remotes join after hosts produced (server late-joiner sync sends existing producers).
for (let r = 0; r < CFG.rooms; r++) {
  for (let k = 1; k < CFG.perRoom; k++) {
    const rm = await newPeerPage(browser, `load-room-${r}`, { role: 'remote', produce: false, consume: true });
    peers.push(rm);
    await new Promise((res) => setTimeout(res, 200));
  }
}
log(`created ${peers.length} peer pages (${CFG.rooms} hosts + ${CFG.rooms * (CFG.perRoom - 1)} remotes); waiting ready...`);
const ready = await waitReady(peers);
log(`ready=${ready}/${peers.length}`);
await new Promise((r) => setTimeout(r, 8000)); // let RTP flow + stats settle
log('initial stats:', JSON.stringify(await collectStats(peers)));

// ---- steady window ----
log(`STEADY ${CFG.steady}s sampling every 5s`);
let elapsed = 0;
while (elapsed < CFG.steady) {
  const m = sample('steady');
  if (elapsed % 30 === 0) log(`steady t=${elapsed}s mem=${m?.toFixed(1)}MiB peak=${peak.toFixed(1)}MiB`);
  await new Promise((r) => setTimeout(r, 5000)); elapsed += 5;
}
const steadyStats = await collectStats(peers);
log('steady stats:', JSON.stringify(steadyStats));

// ---- burst window: add one extra consumer per room (forwarding burst) ----
log(`BURST ${CFG.burst}s — adding ${CFG.rooms} extra consumers`);
const burstPeers = [];
for (let r = 0; r < CFG.rooms; r++) burstPeers.push(await newPeerPage(browser, `load-room-${r}`, { role: 'remote', produce: false, consume: true }));
await waitReady(burstPeers, 30000);
elapsed = 0;
while (elapsed < CFG.burst) {
  const m = sample('burst');
  if (elapsed % 30 === 0) log(`burst t=${elapsed}s mem=${m?.toFixed(1)}MiB peak=${peak.toFixed(1)}MiB`);
  await new Promise((r) => setTimeout(r, 5000)); elapsed += 5;
}
const burstStats = await collectStats([...peers, ...burstPeers]);
log('burst stats:', JSON.stringify(burstStats));

// cgroup peak (kernel-tracked, authoritative)
let cgroupPeakMiB = null;
try {
  const b = execSync(`docker exec ${CFG.container} sh -c 'cat /sys/fs/cgroup/memory.peak 2>/dev/null || cat /sys/fs/cgroup/memory/memory.max_usage_in_bytes 2>/dev/null'`, { encoding: 'utf8' }).trim();
  if (b) cgroupPeakMiB = +(parseInt(b) / 1048576).toFixed(2);
} catch {}

const peakUsed = Math.max(peak, cgroupPeakMiB || 0);
const memLimitMiB = Math.ceil((peakUsed * 1.3) / 512) * 512;
const result = {
  config: CFG, steadyStats, burstStats,
  dockerStatsPeakMiB: +peak.toFixed(2), cgroupPeakMiB,
  peakUsedMiB: +peakUsed.toFixed(2),
  memLimitMiB, memLimitGiB: +(memLimitMiB / 1024).toFixed(2),
  samples,
};
writeFileSync(CFG.out, JSON.stringify(result, null, 2));
log(`RESULT: dockerStatsPeak=${peak.toFixed(1)}MiB cgroupPeak=${cgroupPeakMiB}MiB => mem_limit=${memLimitMiB}MiB (${(memLimitMiB / 1024).toFixed(2)}GiB)`);
log(`wrote ${CFG.out}`);
await browser.close();
httpSrv.close();