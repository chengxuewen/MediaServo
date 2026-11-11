import { describe, it, expect } from 'vitest';
import { classifySfuError, classifyProducerRoute, nextBackoff, mapIceParameters, mapIceCandidates, mapDtlsParameters, wantsProducer } from './sfu-client';

// T0.2 first batch of unit tests: two exported pure functions (W1/W4 resilience cornerstones), semantics pinned against P1 core-swap regressions.

describe('classifySfuError (W4 error classification table)', () => {
  it('auth/authorization family = terminal (sole source of the red banner, D273)', () => {
    for (const code of [4000, 4001, 4002, 4003, 4010, 4011]) {
      expect(classifySfuError(code)).toBe('terminal');
    }
  });
  // P1/T1.2 intentional change (design §1, landing with server 4012): control DC denial joins the terminal family.
  it('4012 (F8 control DC explicit denial) = terminal (T1.2 contract-change pin)', () => {
    expect(classifySfuError(4012)).toBe('terminal');
    expect(classifySfuError(4013)).toBe('terminal'); // T0 fail-closed (JWT signature failure = auth family)
  });
  it('everything else = retry (producer/internal error families included)', () => {
    for (const code of [4031, 5000, 5001, 1006, 0, 4999]) {
      expect(classifySfuError(code)).toBe('retry');
    }
  });
});

describe('wire→mediasoup mapping pure functions (P1 single snake↔camel point)', () => {
  it('mapIceParameters: snake→camel + iceLite always true (mediasoup server design invariant)', () => {
    expect(mapIceParameters({ username_fragment: 'u1', password: 'p1' }))
      .toEqual({ usernameFragment: 'u1', password: 'p1', iceLite: true });
  });
  it('mapIceCandidates: candidateType/candidate_type dual form → type, defaults to host', () => {
    const out = mapIceCandidates([
      { ip: '10.0.0.1', port: 7, protocol: 'udp', foundation: 'f', priority: 1, candidateType: 'host' },
      { ip: '10.0.0.2', port: 8, protocol: 'udp', foundation: 'g', priority: 2, candidate_type: 'srflx' },
      { ip: '10.0.0.3', port: 9, protocol: 'udp', foundation: 'h', priority: 3 },
    ]);
    expect(out.map((c) => c.type)).toEqual(['host', 'srflx', 'host']);
  });
  it('mapDtlsParameters: structure passed through', () => {
    const d = mapDtlsParameters({ fingerprints: [{ algorithm: 'sha-256', value: 'AA' }], role: 'client' });
    expect(d.role).toBe('client');
    expect(d.fingerprints[0]).toEqual({ algorithm: 'sha-256', value: 'AA' });
  });
});

describe('nextBackoff (W1 exponential backoff)', () => {
  it('starts at 1s and grows exponentially up to a 30s cap', () => {
    expect(nextBackoff(1)).toBe(1000);
    expect(nextBackoff(2)).toBe(2000);
    expect(nextBackoff(3)).toBe(4000);
    expect(nextBackoff(5)).toBe(16000);
    expect(nextBackoff(6)).toBe(30000); // 32000 → cap
    expect(nextBackoff(99)).toBe(30000);
  });
  it('custom cap takes effect', () => {
    expect(nextBackoff(4, 5000)).toBe(5000);
    expect(nextBackoff(2, 5000)).toBe(2000);
  });
  it('zero/fractional attempts clamp to 1 (no infinite loop, no zero delay)', () => {
    expect(nextBackoff(0)).toBe(1000);
    expect(nextBackoff(-3)).toBe(1000);
    expect(nextBackoff(1.7)).toBe(1000);
    expect(nextBackoff(2.9)).toBe(2000);
  });
});

describe('wantsProducer (O21 label filter — M1/M2 pure predicate)', () => {
  const M1 = new Set<string>(); // empty set = consume EVERY producer
  const M2 = new Set(['fl', 'fm']);

  it('M1 (no filter set): every producer passes, including unlabeled', () => {
    expect(wantsProducer(undefined, M1)).toBe(true);
    expect(wantsProducer('anything', M1)).toBe(true);
  });

  it('M2: only set-member labels pass; unlabeled producers never match', () => {
    expect(wantsProducer('fl', M2)).toBe(true);
    expect(wantsProducer('cam-x', M2)).toBe(false);
    expect(wantsProducer(undefined, M2)).toBe(false);
  });

  it('M2 boundary: undefined label in a non-empty set is rejected (selective-subscription semantics)', () => {
    // legacy short-form producers (no label) must NOT be consumed by a
    // label-filtered cockpit — the filter is the subscription contract.
    expect(wantsProducer('fm', M2)).toBe(true);
  });
});

describe('classifyProducerRoute (O21-M3 transport routing)', () => {
  const labels = new Set(['left', 'right']); // shared group (M2 selective)
  const isolated = new Set(['front']); // dedicated transport (teleop front camera)

  it('isolated label routes to its own transport (and implicitly joins the subscription)', () => {
    expect(classifyProducerRoute('front', labels, isolated)).toBe('isolated');
    expect(classifyProducerRoute('front', new Set(), isolated)).toBe('isolated'); // M1+isolation
  });

  it('shared-group labels ride the shared transport', () => {
    expect(classifyProducerRoute('left', labels, isolated)).toBe('shared');
  });

  it('everything else is ignored (unlabeled never matches a filter; M2 non-members dropped)', () => {
    expect(classifyProducerRoute(undefined, labels, isolated)).toBe('ignore');
    expect(classifyProducerRoute('cam-x', labels, isolated)).toBe('ignore');
    expect(classifyProducerRoute('fl', new Set(['fl']), isolated)).toBe('shared'); // M2-only posture
  });
});
