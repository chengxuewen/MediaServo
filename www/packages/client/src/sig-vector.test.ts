// P1/T1.3 L1 contract fixtures replayed in two languages: together with crates/mediaservo-common/tests/sig_vector_sf.rs
// it consumes the same JSON set (Rust covers the type-algebra loop; this file covers the TS-side wire shape/default semantics +
// the envelope encode/decode pure functions the P2 control plane will rely on). Path overridable via env MEDIASERVO_SIGVECTOR_DIR.
import { describe, it, expect } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const dir =
  process.env.MEDIASERVO_SIGVECTOR_DIR ??
  fileURLToPath(new URL('../../../../crates/mediaservo-common/tests/sig_vector/sfu/', import.meta.url));

// ── Control envelope wire shape (TS mirror of common::protocol::ControlEnvelope/ControlAck) ──
export interface ControlEnvelope { seq: number; cmd: string; payload?: Record<string, unknown> }
export interface ControlAck { ack: number; result: Record<string, unknown> }

export function encodeEnvelope(env: ControlEnvelope): string {
  return JSON.stringify({ seq: env.seq, cmd: env.cmd, payload: env.payload ?? {} });
}

export function parseEnvelope(text: string): ControlEnvelope {
  const m = JSON.parse(text) as Record<string, unknown>;
  if (typeof m.seq !== 'number' || typeof m.cmd !== 'string') {
    throw new Error('envelope requires a number seq + string cmd');
  }
  return { seq: m.seq, cmd: m.cmd, payload: (m.payload as Record<string, unknown>) ?? {} };
}

export function parseAck(text: string): ControlAck {
  const m = JSON.parse(text) as Record<string, unknown>;
  if (typeof m.ack !== 'number' || typeof m.result !== 'object' || m.result === null) {
    throw new Error('ack requires a number ack + object result');
  }
  return { ack: m.ack, result: m.result as Record<string, unknown> };
}

function get(v: unknown, path: string): unknown {
  let cur: unknown = v;
  for (const seg of path.split('.')) {
    if (Array.isArray(cur)) cur = cur[Number(seg)];
    else if (cur && typeof cur === 'object') cur = (cur as Record<string, unknown>)[seg];
    else return undefined;
  }
  return cur;
}

interface Fixture { name: string; kind: 'signal' | 'envelope' | 'ack'; wire: Record<string, unknown>; checks: Record<string, unknown> }
const files = readdirSync(dir).filter((f: string) => f.endsWith('.json')).sort();

describe('sig_vector/sfu fixture TS replay', () => {
  it('fixture set is non-empty and coverage is sufficient', () => {
    expect(files.length).toBeGreaterThanOrEqual(11);
  });

  for (const f of files) {
    const fx = JSON.parse(readFileSync(`${dir}/${f}`, 'utf8')) as Fixture;
    it(`${fx.name} (${fx.kind})`, () => {
      if (fx.kind === 'envelope') {
        const env = parseEnvelope(JSON.stringify(fx.wire));
        expect(env.seq).toBe(fx.wire.seq);
        expect(env.cmd).toBe(fx.wire.cmd);
        expect(env.payload).toEqual(fx.wire.payload ?? {}); // matches the Rust default {} semantics
        // encode round-trip: encodeEnvelope → parse is idempotent
        expect(parseEnvelope(encodeEnvelope(env))).toEqual(env);
      } else if (fx.kind === 'ack') {
        const ack = parseAck(JSON.stringify(fx.wire));
        expect(ack.ack).toBe(fx.wire.ack);
      }
      // signal surface: TS cannot parse Rust enums — check the dot-path value + type presence (structural pin).
      for (const [path, want] of Object.entries(fx.checks)) {
        expect(get(fx.wire, path), `${fx.name}: ${path}`).toEqual(want);
      }
    });
  }
});
