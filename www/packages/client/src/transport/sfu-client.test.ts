import { describe, it, expect } from 'vitest';
import { classifySfuError, nextBackoff, mapIceParameters, mapIceCandidates, mapDtlsParameters } from './sfu-client';

// T0.2 首批单测：两个导出纯函数（W1/W4 韧性层基石），语义钉住防 P1 换心回归。

describe('classifySfuError (W4 错误分类表)', () => {
  it('auth/授权族 = terminal（红牌唯一源，D273）', () => {
    for (const code of [4000, 4001, 4002, 4003, 4010, 4011]) {
      expect(classifySfuError(code)).toBe('terminal');
    }
  });
  // P1/T1.2 有意变更（design §1，随 server 4012 落地）：控制 DC 拒权入 terminal 族。
  it('4012（F8 控制 DC 显式拒）= terminal（T1.2 契约变更钉）', () => {
    expect(classifySfuError(4012)).toBe('terminal');
  });
  it('其余 = retry（含 producer/内部错误族）', () => {
    for (const code of [4031, 5000, 5001, 1006, 0, 4999]) {
      expect(classifySfuError(code)).toBe('retry');
    }
  });
});

describe('wire→mediasoup 映射纯函数（P1 snake↔camel 单点）', () => {
  it('mapIceParameters: snake→camel + iceLite 恒真（mediasoup server 设计不变量）', () => {
    expect(mapIceParameters({ username_fragment: 'u1', password: 'p1' }))
      .toEqual({ usernameFragment: 'u1', password: 'p1', iceLite: true });
  });
  it('mapIceCandidates: candidateType/candidate_type 双形 → type，缺省 host', () => {
    const out = mapIceCandidates([
      { ip: '10.0.0.1', port: 7, protocol: 'udp', foundation: 'f', priority: 1, candidateType: 'host' },
      { ip: '10.0.0.2', port: 8, protocol: 'udp', foundation: 'g', priority: 2, candidate_type: 'srflx' },
      { ip: '10.0.0.3', port: 9, protocol: 'udp', foundation: 'h', priority: 3 },
    ]);
    expect(out.map((c) => c.type)).toEqual(['host', 'srflx', 'host']);
  });
  it('mapDtlsParameters: 结构透传', () => {
    const d = mapDtlsParameters({ fingerprints: [{ algorithm: 'sha-256', value: 'AA' }], role: 'client' });
    expect(d.role).toBe('client');
    expect(d.fingerprints[0]).toEqual({ algorithm: 'sha-256', value: 'AA' });
  });
});

describe('nextBackoff (W1 指数退避)', () => {
  it('1s 起步指数增长至 30s 封顶', () => {
    expect(nextBackoff(1)).toBe(1000);
    expect(nextBackoff(2)).toBe(2000);
    expect(nextBackoff(3)).toBe(4000);
    expect(nextBackoff(5)).toBe(16000);
    expect(nextBackoff(6)).toBe(30000); // 32000 → cap
    expect(nextBackoff(99)).toBe(30000);
  });
  it('自定义 cap 生效', () => {
    expect(nextBackoff(4, 5000)).toBe(5000);
    expect(nextBackoff(2, 5000)).toBe(2000);
  });
  it('零/小数 attempt 钳到 1（不死循环不零间隔）', () => {
    expect(nextBackoff(0)).toBe(1000);
    expect(nextBackoff(-3)).toBe(1000);
    expect(nextBackoff(1.7)).toBe(1000);
    expect(nextBackoff(2.9)).toBe(2000);
  });
});
