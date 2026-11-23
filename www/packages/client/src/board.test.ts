import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BoardStore, type BoardSnapshot } from './board';

// Minimal WebSocket double: captures sends, lets tests drive the lifecycle.
class FakeWS {
  static instances: FakeWS[] = [];
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  sent: string[] = [];
  readyState: number = FakeWS.CONNECTING;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((e: unknown) => void) | null = null;
  constructor(
    public url: string,
    public protocols?: string[],
  ) {
    FakeWS.instances.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.readyState = FakeWS.CLOSED;
    this.onclose?.();
  }
  // test helpers
  open() {
    this.readyState = FakeWS.OPEN;
    this.onopen?.();
  }
  msg(obj: unknown) {
    this.onmessage?.({ data: JSON.stringify(obj) });
  }
  raw(data: string) {
    this.onmessage?.({ data });
  }
}

function lastWS(): FakeWS {
  return FakeWS.instances[FakeWS.instances.length - 1];
}

let store: BoardStore | null = null;

beforeEach(() => {
  FakeWS.instances = [];
  vi.useFakeTimers();
});
afterEach(() => {
  store?.close();
  vi.useRealTimers();
});

function makeStore(rooms: string[] = ['r1']): BoardStore {
  return new BoardStore({
    rooms,
    token: 'test-token',
    wsFactory: (url, p) => new FakeWS(url, p) as unknown as WebSocket,
  });
}

describe('BoardStore (T3)', () => {
  it('opens one signaling ws per room and sends room_join on open (JWT subprotocol)', () => {
    store = makeStore(['r1', 'r2']);
    expect(FakeWS.instances).toHaveLength(2);
    const ws = lastWS();
    expect(ws.url.endsWith('/ws')).toBe(true);
    expect(ws.protocols).toEqual(['test-token']); // PIT-49: raw JWT subprotocol
    ws.open();
    const join = ws.sent.map((s) => JSON.parse(s) as Record<string, unknown>).find((m) => m.type === 'room_join');
    expect(join).toMatchObject({ type: 'room_join', room_id: 'r2', protocol: 4 });
  });

  it('room_joined captures peer_id and clears stale', () => {
    store = makeStore(['r1']);
    const ws = FakeWS.instances[0];
    let snap: BoardSnapshot | null = null;
    store.subscribe((s) => (snap = s));
    ws.open();
    ws.msg({ type: 'room_joined', room_id: 'r1', peer_id: 'dispatcher-20a95866', protocol: 4 });
    expect(snap!.rooms.r1.peerId).toBe('dispatcher-20a95866');
    expect(snap!.rooms.r1.stale).toBe(false);
    expect(snap!.rooms.r1.conn).toBe('connected');
  });

  it('floor_state snapshot lands in lastEvent; older generations are discarded', () => {
    store = makeStore(['r1']);
    const ws = FakeWS.instances[0];
    let snap: BoardSnapshot | null = null;
    store.subscribe((s) => (snap = s));
    ws.open();
    ws.msg({ type: 'room_joined', room_id: 'r1', peer_id: 'p-x', protocol: 4 });
    ws.msg({
      type: 'floor_state',
      room_id: 'r1',
      snapshot: { mode: 'exclusive', grants: ['p1'], muted: [] },
      generation: 7,
    });
    expect(snap!.rooms.r1.lastEvent).toMatchObject({ type: 'floor_state', generation: 7 });
    ws.msg({
      type: 'floor_granted',
      room_id: 'r1',
      grants: ['p1', 'p2'],
      generation: 9,
    });
    expect(snap!.rooms.r1.lastEvent).toMatchObject({ type: 'floor_granted', generation: 9 });
    // stale (older than 9) — silently dropped
    ws.msg({
      type: 'floor_state',
      room_id: 'r1',
      snapshot: { mode: 'exclusive', grants: [], muted: [] },
      generation: 8,
    });
    expect(snap!.rooms.r1.lastEvent).toMatchObject({ type: 'floor_granted', generation: 9 });
  });

  it('disconnect marks the room stale/reconnecting (never idle) and reconnects with backoff', () => {
    store = makeStore(['r1']);
    const ws = FakeWS.instances[0];
    let snap: BoardSnapshot | null = null;
    store.subscribe((s) => (snap = s));
    ws.open();
    ws.msg({ type: 'room_joined', room_id: 'r1', peer_id: 'p-x', protocol: 4 });
    ws.close(); // server dropped us
    expect(snap!.rooms.r1.stale).toBe(true);
    expect(snap!.rooms.r1.conn).toBe('reconnecting');
    expect(snap!.rooms.r1.peerId).toBeNull();
    // backoff: first retry after base 500ms
    vi.advanceTimersByTime(500);
    expect(FakeWS.instances).toHaveLength(2); // reopened
  });

  it('sendCommand routes to the open room ws; throws when not connected / unknown room', () => {
    store = makeStore(['r1']);
    const ws = FakeWS.instances[0];
    ws.open();
    ws.msg({ type: 'room_joined', room_id: 'r1', peer_id: 'p-x', protocol: 4 }); // reach 'connected'
    store.sendCommand('r1', { type: 'floor_release' });
    expect(ws.sent.some((s) => s.includes('floor_release'))).toBe(true);
    expect(() => store!.sendCommand('nope', { type: 'floor_release' })).toThrow();
    // disconnected again → guarded (state left 'reconnecting')
    ws.close();
    expect(() => store!.sendCommand('r1', { type: 'floor_release' })).toThrow();
  });

  it('close() stops reconnects', () => {
    store = makeStore(['r1']);
    const ws = FakeWS.instances[0];
    ws.close(); // schedules reconnect
    store.close();
    const count = FakeWS.instances.length;
    vi.advanceTimersByTime(10000);
    expect(FakeWS.instances).toHaveLength(count); // no reopen after user close
  });

  it('subscribe fires immediately with the current snapshot', () => {
    store = makeStore(['r1']);
    let snap: BoardSnapshot | null = null;
    const unsub = store.subscribe((s) => (snap = s));
    expect(snap!.rooms.r1.roomId).toBe('r1');
    unsub();
  });

  it('roster: room_peers snapshot lands, peer_joined upserts, room_leave removes', () => {
    store = makeStore(['r1']);
    const ws = FakeWS.instances[0];
    let snap: BoardSnapshot | null = null;
    store.subscribe((s) => (snap = s));
    ws.open();
    // join-time snapshot with two members (identity pruned for one, full for the other)
    ws.msg({
      type: 'room_peers',
      room_id: 'r1',
      peers: [
        { peer_id: 'carol-a1', username: 'carol', role: 'operator' },
        { peer_id: 'ms-car1-x', device_id: 'ms-car1' },
      ],
    });
    expect(Object.keys(snap!.roster.r1)).toHaveLength(2);
    expect(snap!.roster.r1['carol-a1']).toMatchObject({ username: 'carol', role: 'operator' });
    expect(snap!.roster.r1['ms-car1-x']).toMatchObject({ device_id: 'ms-car1' });
    // incremental join: presence-only (identity kept if previously known)
    ws.msg({ type: 'peer_joined', room_id: 'r1', peer_id: 'dave-b2' });
    expect(snap!.roster.r1['dave-b2']).toEqual({ peer_id: 'dave-b2' });
    // leave removes
    ws.msg({ type: 'room_leave', room_id: 'r1', peer_id: 'carol-a1' });
    expect(snap!.roster.r1['carol-a1']).toBeUndefined();
  });
});
