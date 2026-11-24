#!/usr/bin/env node
// S1-L1 host push-stream one-shot gate (admin-prototype-restyle PLAN §5b.2).
// Local acceptance gate, NOT a CI gate (CI has no binary / build too slow).
//
//   node scripts/e2e-stream.mjs          (repo root; cargo on PATH = pixi env)
//
// Chain: ensure host binary (missing/stale -> cargo build) -> start or reuse
// server(9800) -> spawn or reuse host-legacy (PSK) -> poll GET /api/admin/sfu/rooms
// until producers >= 1 (timeout = red) -> print evidence -> clean up ONLY
// self-started PIDs (PIT-277; E5: an existing host is reused, never killed).
// Exit 0 = green, 1 = red. Failure paths tear down too.
import { spawn, spawnSync, execSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const HOST_BIN = path.join(ROOT, 'target', 'debug', 'host-legacy');
const SERVER_BIN = path.join(ROOT, 'target', 'debug', 'mediaservo-server');
const SERVER_CFG = '/tmp/ms-e2e/server.yaml';
const BASE = 'http://127.0.0.1:9800';
const SCRATCH = path.join(ROOT, '.cache', 'tmp');
const HOST_LOG = path.join(SCRATCH, 'e2e-stream-host.log');
const SERVER_LOG = path.join(SCRATCH, 'e2e-stream-server.log');
const POLL_MS = 2000;
const DEADLINE_MS = 60_000; // own producer takes ~12 s to start (sfu.spec note)

fs.mkdirSync(SCRATCH, { recursive: true });

let selfHost = null;
let selfServer = null;
let hostLogFd = null;

function cleanup() {
  // E5/PIT-277: only self-started PIDs die here; reused processes stay alive.
  for (const child of [selfHost, selfServer]) {
    if (child && child.exitCode === null && !child.killed) {
      try { child.kill('SIGTERM'); } catch { /* already gone */ }
    }
  }
  if (hostLogFd !== null) { try { fs.closeSync(hostLogFd); } catch { /* closed */ } hostLogFd = null; }
}
process.on('SIGINT', () => { cleanup(); process.exit(1); });
process.on('SIGTERM', () => { cleanup(); process.exit(1); });
// SIGHUP (terminal close) previously fell through to the default action = node dies
// without cleanup, orphaning the self-started server (the 11-24 port-9800 residue incident).
process.on('SIGHUP', () => { cleanup(); process.exit(1); });
// exit hook covers process.exit paths / normal termination (cleanup is sync-only).
// SIGKILL cannot be hooked -- msrtc-server start's pid-level residue takeover is the backstop.
process.on('exit', cleanup);

function fail(msg) {
  console.error(`S1-L1 RED: ${msg}`);
  cleanup();
  process.exit(1);
}

function tcpUp(port, timeoutMs = 1500) {
  return new Promise((resolve) => {
    const sock = net.connect({ host: '127.0.0.1', port });
    const done = (up) => { sock.destroy(); resolve(up); };
    sock.setTimeout(timeoutMs);
    sock.once('connect', () => done(true));
    sock.once('timeout', () => done(false));
    sock.once('error', () => done(false));
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function srcNewerThan(bin) {
  const binM = fs.statSync(bin).mtimeMs;
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { if (walk(p)) return true; }
      else if (e.name.endsWith('.rs') && fs.statSync(p).mtimeMs > binM) return true;
    }
    return false;
  };
  return walk(path.join(ROOT, 'crates', 'mediaservo-host', 'src'));
}

function ensureHostBinary() {
  if (fs.existsSync(HOST_BIN) && !srcNewerThan(HOST_BIN)) {
    console.log(`host binary fresh: ${HOST_BIN}`);
    return;
  }
  console.log('host binary missing/stale -> cargo build -p mediaservo-host --bin host-legacy');
  const r = spawnSync('cargo', ['build', '-p', 'mediaservo-host', '--bin', 'host-legacy'], {
    cwd: ROOT,
    stdio: 'inherit',
    env: { ...process.env, MEDIASOUP_LOCAL_DEV: '1' },
  });
  if (r.status !== 0) fail(`cargo build host-legacy exited ${r.status}`);
}

async function ensureServer() {
  if (await tcpUp(9800)) { console.log('server 9800 already up -> reuse'); return; }
  if (!fs.existsSync(SERVER_BIN)) fail(`server down and ${SERVER_BIN} missing (build it first)`);
  if (!fs.existsSync(SERVER_CFG)) fail(`server down and fixture ${SERVER_CFG} missing (source-stack recipe, PLAN §5b.5)`);
  console.log('server down -> self-start with fixture config');
  const fd = fs.openSync(SERVER_LOG, 'a');
  selfServer = spawn(SERVER_BIN, ['--config', SERVER_CFG], { cwd: ROOT, stdio: ['ignore', fd, fd] });
  fs.closeSync(fd);
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (await tcpUp(9800)) { console.log(`self-started server pid=${selfServer.pid}`); return; }
    await sleep(500);
  }
  fail('self-started server did not listen on 9800 within 30 s (see e2e-stream-server.log)');
}

async function adminToken() {
  const res = await fetch(`${BASE}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: 'admin', password: 'admin123' }),
  });
  if (!res.ok) fail(`admin login HTTP ${res.status} (dev credentials required, allow_dev_credentials)`);
  const body = await res.json();
  if (!body.token) fail('admin login response has no token');
  return body.token;
}

function existingHostPids() {
  try {
    // -x (exact comm name): -f would self-match the probing sh -c cmdline
    return execSync('pgrep -x host-legacy || true', { encoding: 'utf8' })
      .trim().split('\n').filter(Boolean).map(Number);
  } catch { return []; }
}

async function ensureHost() {
  const pids = existingHostPids();
  if (pids.length > 0) {
    console.log(`existing host pid(s) ${pids.join(',')} -> reuse (E5: never killed)`);
    return;
  }
  hostLogFd = fs.openSync(HOST_LOG, 'w'); // per-run evidence, no cross-run mixing
  // RUST_LOG=info: producer/ICE evidence lines (default env filter is error-only)
  selfHost = spawn(HOST_BIN, [], {
    cwd: ROOT,
    stdio: ['ignore', hostLogFd, hostLogFd],
    env: { ...process.env, RUST_LOG: process.env.RUST_LOG ?? 'info' },
  });
  console.log(`self-started host pid=${selfHost.pid} (log: ${HOST_LOG})`);
}

async function pollProducers(token) {
  const deadline = Date.now() + DEADLINE_MS;
  const t0 = Date.now();
  while (Date.now() < deadline) {
    const res = await fetch(`${BASE}/api/admin/sfu/rooms`, { headers: { Authorization: `Bearer ${token}` } });
    if (res.ok) {
      const body = await res.json();
      const hit = (body.rooms ?? []).find((r) => (r.producers ?? 0) >= 1);
      if (hit) return { room: hit.room_id, producers: hit.producers, elapsedMs: Date.now() - t0 };
    }
    await sleep(POLL_MS);
  }
  fail(`no room with producers >= 1 within ${DEADLINE_MS / 1000} s (host log: ${HOST_LOG})`);
}

function printLogEvidence() {
  if (!fs.existsSync(HOST_LOG)) return;
  const lines = fs.readFileSync(HOST_LOG, 'utf8').split('\n')
    .filter((l) => /producer|ICE|connected|sfu/i.test(l));
  console.log('--- host log evidence (producer/ICE/connected, last 6) ---');
  for (const l of lines.slice(-6)) console.log(l);
}

try {
  ensureHostBinary();
  await ensureServer();
  const token = await adminToken();
  await ensureHost();
  const ev = await pollProducers(token);
  console.log(`S1-L1 GREEN: room=${ev.room} producers=${ev.producers} elapsed=${ev.elapsedMs}ms`);
  printLogEvidence();
  cleanup();
  process.exit(0);
} catch (e) {
  fail(e?.message ?? String(e));
}