import { describe, expect, it } from 'vitest';
import {
  FloorClient,
  generationIsTsSafe,
  isStaleGeneration,
  type FloorCommand,
  type FloorServerEvent,
} from './floor';

function makeClient() {
  const sent: FloorCommand[] = [];
  const events: FloorServerEvent[] = [];
  const client = new FloorClient({
    send: (cmd) => sent.push(cmd),
    roomId: 'talk-car1',
    onEvent: (ev) => events.push(ev),
  });
  return { client, sent, events };
}

describe('FloorClient commands', () => {
  it('floor_request omits optional keys when undefined (wire contract)', () => {
    const { client, sent } = makeClient();
    client.request();
    expect(sent).toEqual([{ type: 'floor_request' }]);
  });

  it('floor_request carries priority/preempt when provided', () => {
    const { client, sent } = makeClient();
    client.request(7, true);
    expect(sent).toEqual([{ type: 'floor_request', priority: 7, preempt: true }]);
  });

  it('release/resync/mode_set/mute_set wire shapes', () => {
    const { client, sent } = makeClient();
    client.release();
    client.resync();
    client.modeSet('hybrid');
    client.muteSet('p3', true);
    expect(sent).toEqual([
      { type: 'floor_release' },
      { type: 'floor_resync' },
      { type: 'floor_mode_set', mode: 'hybrid' },
      { type: 'floor_mute_set', target_peer_id: 'p3', muted: true },
    ]);
  });
});

describe('FloorClient server events', () => {
  it('parses floor_granted, updates generation and grants', () => {
    const { client, events } = makeClient();
    const consumed = client.handleWire(
      '{"type":"floor_granted","room_id":"talk-car1","grants":["a"],"generation":6}',
    );
    expect(consumed).toBe(true);
    expect(events).toEqual([
      { type: 'floor_granted', room_id: 'talk-car1', grants: ['a'], generation: 6 },
    ]);
    expect(client.generation).toBe(6);
    expect(client.grants).toEqual(['a']);
  });

  it('discards stale frames (older generation) without emitting', () => {
    const { client, events } = makeClient();
    expect(client.handleWire('{"type":"floor_granted","room_id":"r","grants":["a"],"generation":9}')).toBe(true);
    events.length = 0;
    // stale: generation 5 < 9 — swallowed silently (floor-family => consumed=true)
    expect(client.handleWire('{"type":"floor_queued","room_id":"r","position":1,"generation":5}')).toBe(true);
    expect(events).toEqual([]);
    expect(client.generation).toBe(9);
  });

  it('floor_idle clears the grant set', () => {
    const { client } = makeClient();
    client.handleWire('{"type":"floor_granted","room_id":"r","grants":["a"],"generation":2}');
    expect(client.grants).toEqual(['a']);
    client.handleWire('{"type":"floor_idle","room_id":"r","reason":"timeout","generation":3}');
    expect(client.grants).toEqual([]);
    expect(client.generation).toBe(3);
  });

  it('floor_state snapshot updates grants and is the late-joiner replay', () => {
    const { client, events } = makeClient();
    const consumed = client.handleWire(
      '{"type":"floor_state","room_id":"r","generation":4,"snapshot":{"mode":"exclusive","grants":["x"],"muted":["y"],"queue":[{"peer":"z","priority":10}]}}',
    );
    expect(consumed).toBe(true);
    expect(client.grants).toEqual(['x']);
    const st = events[0] as Extract<FloorServerEvent, { type: 'floor_state' }>;
    expect(st.snapshot.queue).toEqual([{ peer: 'z', priority: 10 }]);
  });

  it('returns false for non-floor frames (SFU handleMessage keeps them)', () => {
    const { client, events } = makeClient();
    expect(client.handleWire('{"type":"room_joined","room_id":"r"}')).toBe(false);
    expect(client.handleWire('not json at all')).toBe(false);
    expect(events).toEqual([]);
  });
});

describe('O29 generation safety', () => {
  it('counter-scale values are plain-Number-safe', () => {
    expect(generationIsTsSafe(1)).toBe(true);
    expect(generationIsTsSafe(1_000_000)).toBe(true);
    expect(generationIsTsSafe(Number.MAX_SAFE_INTEGER)).toBe(true);
  });

  it('isStaleGeneration semantics: equal is not stale, older is', () => {
    expect(isStaleGeneration(5, 5)).toBe(false);
    expect(isStaleGeneration(4, 5)).toBe(true);
  });
});
