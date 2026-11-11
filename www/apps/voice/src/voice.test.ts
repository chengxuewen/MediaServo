import { describe, expect, it } from 'vitest';
import { initialFloorUi, reduceFloorUi, cueFor, talkPosture } from './state';
import type { FloorServerEvent } from '@mediaservo/client';

const ev = (o: Record<string, unknown>) => o as unknown as FloorServerEvent;

describe('reduceFloorUi (M4 state machine — pure)', () => {
  it('granted updates grants + generation and clears deniedReason', () => {
    let s = initialFloorUi('me');
    s = reduceFloorUi(s, ev({ type: 'floor_denied', room_id: 'r', reason: 'busy', generation: 3 }));
    expect(s.deniedReason).toBe('busy');
    s = reduceFloorUi(s, ev({ type: 'floor_granted', room_id: 'r', grants: ['me'], generation: 4 }));
    expect(s.grants).toEqual(['me']);
    expect(s.generation).toBe(4);
    expect(s.deniedReason).toBeUndefined();
    expect(talkPosture(s)).toBe('granted');
  });

  it('taken removes the taker from grants (preempt view)', () => {
    let s = initialFloorUi('me');
    s = reduceFloorUi(s, ev({ type: 'floor_granted', room_id: 'r', grants: ['other'], generation: 1 }));
    s = reduceFloorUi(s, ev({ type: 'floor_taken', room_id: 'r', by: 'other', generation: 2 }));
    expect(s.grants).toEqual([]);
    expect(talkPosture(s)).toBe('idle');
  });

  it('floor_state is the late-joiner replay (snapshot authority)', () => {
    let s = initialFloorUi('me');
    s = reduceFloorUi(s, ev({
      type: 'floor_state', room_id: 'r', generation: 9,
      snapshot: { mode: 'hybrid', grants: ['a'], muted: ['b'], queue: [{ peer: 'me', priority: 8 }] },
    }));
    expect(s.mode).toBe('hybrid');
    expect(s.grants).toEqual(['a']);
    expect(s.muted).toEqual(['b']);
    expect(s.queue).toEqual([{ peer: 'me', priority: 8 }]);
  });

  it('idle clears grants and the queue', () => {
    let s = initialFloorUi('me');
    s = reduceFloorUi(s, ev({ type: 'floor_granted', room_id: 'r', grants: ['me'], generation: 1 }));
    s = reduceFloorUi(s, ev({ type: 'floor_idle', room_id: 'r', generation: 2 }));
    expect(s.grants).toEqual([]);
    expect(s.queue).toEqual([]);
  });
});

describe('cueFor (M4 cue derivation)', () => {
  const base = initialFloorUi('me');

  it('granted fires only for self', () => {
    expect(cueFor(base, ev({ type: 'floor_granted', room_id: 'r', grants: ['me'], generation: 1 }))).toBe('granted');
    expect(cueFor(base, ev({ type: 'floor_granted', room_id: 'r', grants: ['other'], generation: 1 }))).toBeNull();
  });

  it('denied always fires; released fires only when I held the floor', () => {
    expect(cueFor(base, ev({ type: 'floor_denied', room_id: 'r', reason: 'busy', generation: 1 }))).toBe('denied');
    const held = reduceFloorUi(base, ev({ type: 'floor_granted', room_id: 'r', grants: ['me'], generation: 1 }));
    expect(cueFor(held, ev({ type: 'floor_idle', room_id: 'r', generation: 2 }))).toBe('released');
    expect(cueFor(base, ev({ type: 'floor_idle', room_id: 'r', generation: 2 }))).toBeNull();
  });

  it('no selfPeer → silent (pre-connect events never beep)', () => {
    const anon = initialFloorUi();
    expect(cueFor(anon, ev({ type: 'floor_granted', room_id: 'r', grants: ['me'], generation: 1 }))).toBeNull();
  });
});
