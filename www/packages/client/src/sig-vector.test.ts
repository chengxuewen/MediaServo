// P1/T1.3 L1 契约夹具双语言回放：与 crates/mediaservo-common/tests/sig_vector_sf.rs
// 消费同一 JSON 集（Rust 管类型代数闭环，本文件管 TS 侧 wire 形/默认值语义 +
// P2 控制面将依赖的信封编解码纯函数）。路径可用 env MEDIASERVO_SIGVECTOR_DIR 覆盖。
import { describe, it, expect } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const dir =
  process.env.MEDIASERVO_SIGVECTOR_DIR ??
  fileURLToPath(new URL('../../../../crates/mediaservo-common/tests/sig_vector/sfu/', import.meta.url));

// ── 控制信封线形（= common::protocol::ControlEnvelope/ControlAck 的 TS 镜像）──
export interface ControlEnvelope { seq: number; cmd: string; payload?: Record<string, unknown> }
export interface ControlAck { ack: number; result: Record<string, unknown> }

export function encodeEnvelope(env: ControlEnvelope): string {
  return JSON.stringify({ seq: env.seq, cmd: env.cmd, payload: env.payload ?? {} });
}

export function parseEnvelope(text: string): ControlEnvelope {
  const m = JSON.parse(text) as Record<string, unknown>;
  if (typeof m.seq !== 'number' || typeof m.cmd !== 'string') {
    throw new Error('envelope 需要 number seq + string cmd');
  }
  return { seq: m.seq, cmd: m.cmd, payload: (m.payload as Record<string, unknown>) ?? {} };
}

export function parseAck(text: string): ControlAck {
  const m = JSON.parse(text) as Record<string, unknown>;
  if (typeof m.ack !== 'number' || typeof m.result !== 'object' || m.result === null) {
    throw new Error('ack 需要 number ack + object result');
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

describe('sig_vector/sfu 夹具 TS 回放', () => {
  it('夹具集非空且覆盖面达标', () => {
    expect(files.length).toBeGreaterThanOrEqual(11);
  });

  for (const f of files) {
    const fx = JSON.parse(readFileSync(`${dir}/${f}`, 'utf8')) as Fixture;
    it(`${fx.name} (${fx.kind})`, () => {
      if (fx.kind === 'envelope') {
        const env = parseEnvelope(JSON.stringify(fx.wire));
        expect(env.seq).toBe(fx.wire.seq);
        expect(env.cmd).toBe(fx.wire.cmd);
        expect(env.payload).toEqual(fx.wire.payload ?? {}); // Rust default {} 语义对齐
        // 编码闭环：encodeEnvelope → parse 幂等
        expect(parseEnvelope(encodeEnvelope(env))).toEqual(env);
      } else if (fx.kind === 'ack') {
        const ack = parseAck(JSON.stringify(fx.wire));
        expect(ack.ack).toBe(fx.wire.ack);
      }
      // signal 面：TS 无 Rust 枚举可解析——检查 dot-path 值 + type 存在（结构钉）。
      for (const [path, want] of Object.entries(fx.checks)) {
        expect(get(fx.wire, path), `${fx.name}: ${path}`).toEqual(want);
      }
    });
  }
});
