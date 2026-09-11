import { describe, it, expect } from 'vitest';
import { classifySfuError, nextBackoff } from './sfu-client';

// T0.2 首批单测：两个导出纯函数（W1/W4 韧性层基石），语义钉住防 P1 换心回归。

describe('classifySfuError (W4 错误分类表)', () => {
  it('auth 族 = terminal（红牌唯一源，D273）', () => {
    for (const code of [4000, 4001, 4002, 4003, 4010, 4011]) {
      expect(classifySfuError(code)).toBe('terminal');
    }
  });
  it('其余 = retry（含 producer/内部错误族）', () => {
    for (const code of [4031, 5000, 5001, 1006, 0, 4999, 4012]) {
      expect(classifySfuError(code)).toBe('retry');
    }
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
