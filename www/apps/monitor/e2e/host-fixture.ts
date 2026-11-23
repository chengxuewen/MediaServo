import { test as base, expect } from '@playwright/test';
import { spawn, execSync } from 'node:child_process';
import path from 'node:path';

// S13雏形 host fixture (P3 slice ③a): ensure a live video producer for the wall test.
// E5/PIT-277: reuse an existing host (never kill it); only a self-started host is torn
// down. Mirrors scripts/e2e-stream.mjs (S1-L1) — host-legacy is PSK-only and produces
// into a device room (test-room); the monitor tree reads it from /sfu/stats (B5).
const ROOT = path.resolve(process.cwd(), '../../..');
const HOST_BIN = path.join(ROOT, 'target', 'debug', 'host-legacy');
const BASE = 'http://127.0.0.1:9800';

async function adminToken(): Promise<string> {
  const res = await fetch(`${BASE}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: 'admin', password: 'admin123' }),
  });
  if (!res.ok) throw new Error(`admin login HTTP ${res.status} (dev credentials required)`);
  const body = await res.json();
  if (!body.token) throw new Error('admin login response has no token');
  return body.token;
}

function existingHost(): boolean {
  try {
    return execSync('pgrep -x host-legacy || true', { encoding: 'utf8' }).trim().length > 0;
  } catch {
    return false;
  }
}

async function producerRoom(token: string): Promise<string | null> {
  const res = await fetch(`${BASE}/api/admin/sfu/stats`, { headers: { Authorization: `Bearer ${token}` } });
  if (!res.ok) return null;
  const body = await res.json();
  const hit = (body.streams ?? []).find(
    (s: { role: string; kind: string; room: string }) => s.role === 'producer' && s.kind === 'video',
  );
  return hit ? hit.room : null;
}

export const test = base.extend<{ hostProducer: { room: string } }>({
  hostProducer: async ({}, use) => {
    const token = await adminToken();
    let room = await producerRoom(token);
    let selfPid: number | null = null;
    if (!room && !existingHost()) {
      const child = spawn(HOST_BIN, [], { cwd: ROOT, stdio: 'ignore', env: { ...process.env, RUST_LOG: 'info' } });
      selfPid = child.pid ?? null;
    }
    const deadline = Date.now() + 60_000; // own producer takes ~4-12s to start (S1-L1 note)
    while (!room && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 2000));
      room = await producerRoom(token);
    }
    if (!room) {
      if (selfPid) { try { process.kill(selfPid, 'SIGTERM'); } catch { /* already gone */ } }
      throw new Error('no live video producer within 60s (host-legacy ICE failure? run scripts/e2e-stream.mjs)');
    }
    await use({ room });
    if (selfPid) { try { process.kill(selfPid, 'SIGTERM'); } catch { /* already gone */ } }
  },
});

export { expect };