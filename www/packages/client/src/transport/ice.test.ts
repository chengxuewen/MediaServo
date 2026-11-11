// P4-5: scoped RTCIceServer injection (coturn short-term credentials).
// Node-environment unit tests — `window` is stubbed on globalThis because the
// helper reads window.RTCPeerConnection (the mediasoup-client handler does the
// same at transport-create time).
import { describe, it, expect, beforeEach } from 'vitest';
import { withIceServers } from './sfu-client';

/* eslint-disable @typescript-eslint/no-explicit-any */
const g = globalThis as any;

class FakePC {
  static last: FakePC | null = null;
  cfg: any;
  constructor(cfg: any) {
    this.cfg = cfg;
    FakePC.last = this;
  }
}

describe('withIceServers (P4-5 coturn credentials)', () => {
  beforeEach(() => {
    g.window = { RTCPeerConnection: FakePC };
    FakePC.last = null;
  });

  it('patches the PC config inside the window and restores after', () => {
    const pc = withIceServers([{ urls: 'turn:example.com' } as any], () =>
      new (g.window.RTCPeerConnection as any)({ iceParameters: 'x' }),
    );
    expect(pc.cfg.iceServers).toEqual([{ urls: 'turn:example.com' }]);
    expect(g.window.RTCPeerConnection).toBe(FakePC); // global restored
  });

  it('merges with caller-provided iceServers (caller first, injection last)', () => {
    const pc = withIceServers([{ urls: 'stun:a' } as any], () =>
      new (g.window.RTCPeerConnection as any)({ iceServers: [{ urls: 'stun:b' }] }),
    );
    expect(pc.cfg.iceServers).toEqual([{ urls: 'stun:b' }, { urls: 'stun:a' }]);
  });

  it('no-op without servers: fn runs, global untouched', () => {
    const before = g.window.RTCPeerConnection;
    const result = withIceServers(undefined, () => 'unchanged');
    expect(result).toBe('unchanged');
    expect(g.window.RTCPeerConnection).toBe(before);
  });

  it('restores the global even when fn throws', () => {
    expect(() =>
      withIceServers([{ urls: 'turn:example.com' } as any], () => {
        throw new Error('boom');
      }),
    ).toThrow('boom');
    expect(g.window.RTCPeerConnection).toBe(FakePC);
  });
});
