// BoardStore (talk-app T3, ruling I2): signaling-only multi-room floor board
// core — framework-agnostic (headless-friendly, D4); React wrappers live in
// talk-ui, not here.
//
// Transport shape (spike-validated 2026-11-17, §3g): ONE signaling-only WS per
// room (same-account N-conn × N talk rooms works; single-connection repeated
// room_join is NOT supported — O17). Interface-first on purpose: views depend
// only on BoardStore, so a server-side aggregation adapter can replace the
// transport later WITHOUT touching views. Written upgrade trigger (I2): room
// count > ~12 or a second multi-room page appears.
//
// Wire facts reused here (floor.ts header + spike):
// - JWT travels as the sec-websocket-protocol subprotocol (raw token, PIT-49).
// - After open: {type:'room_join', room_id, peer_role, protocol:4}; the server
//   answers room_joined (carries our peer_id — embeds the username) and pushes
//   a floor_state snapshot for late joiners; floor events fan out room-wide.
// - generation is monotonic per server lifetime; stale frames are discarded by
//   FloorClient (O29).
// - Discipline (K): when a room's connection is down the UI must show
//   unknown — never idle. BoardStore marks such rooms `stale: true`.

import { FloorClient, type FloorCommand, type FloorServerEvent } from './floor';
import { getToken } from './auth';

export type RoomConnState = 'connecting' | 'connected' | 'reconnecting' | 'closed';

export interface BoardRoomSnapshot {
  roomId: string;
  conn: RoomConnState;
  /** True whenever the live connection is down — UI must render unknown. */
  stale: boolean;
  /** Latest stale-filtered floor frame (undefined before the first one). */
  lastEvent: FloorServerEvent | null;
  /** Our peer id on this connection (server-assigned; embeds the username). */
  peerId: string | null;
}

export interface BoardSnapshot {
  rooms: Record<string, BoardRoomSnapshot>;
  /** SB-3 roster (T6): per-room presence map, fed by room_peers / peer_joined /
   *  room_leave. Identity (username/role) appears only when the server's
   *  dispatch-scope pruning sent it; broadcasts are presence-only by contract. */
  roster: Record<string, Record<string, WirePeerPresence>>;
  /** Monotonic change counter — cheap change detection for subscribers. */
  version: number;
}

/** SB-3 roster presence (wire mirror of mediaservo-common PeerPresence). */
export interface WirePeerPresence {
  peer_id: string;
  username?: string;
  role?: string;
  device_id?: string;
}

export interface BoardStoreOptions {
  /** WS base, e.g. 'http://127.0.0.1:9800'. Default: same origin (''). */
  baseUrl?: string;
  /** Auth token; defaults to auth getToken() (localStorage). */
  token?: string;
  /** Rooms to watch (one signaling connection each). More via watch(). */
  rooms: string[];
  /** Reconnect backoff base in ms (doubles per attempt, caps at 8s). */
  reconnectBaseMs?: number;
  /** Test/transport injection. Defaults to the browser WebSocket. */
  wsFactory?: (url: string, protocols: string[]) => WebSocket;
}

interface RoomConn {
  roomId: string;
  state: RoomConnState;
  ws: WebSocket | null;
  floor: FloorClient;
  peerId: string | null;
  lastEvent: FloorServerEvent | null;
  attempt: number;
  closedByUser: boolean;
  reconnectTimer: ReturnType<typeof setTimeout> | null;
}

const MAX_BACKOFF_MS = 8000;

export class BoardStore {
  private readonly conns = new Map<string, RoomConn>();
  private readonly rosters = new Map<string, Map<string, WirePeerPresence>>();
  private cached: BoardSnapshot | null = null;
  private readonly listeners = new Set<(s: BoardSnapshot) => void>();
  private version = 0;
  private readonly baseUrl: string;
  private readonly token: string | null;
  private readonly reconnectBaseMs: number;
  private readonly wsFactory: (url: string, protocols: string[]) => WebSocket;

  constructor(opts: BoardStoreOptions) {
    this.baseUrl = opts.baseUrl ?? '';
    this.reconnectBaseMs = opts.reconnectBaseMs ?? 500;
    this.token = opts.token ?? getToken();
    this.wsFactory = opts.wsFactory ?? ((url, protocols) => new WebSocket(url, protocols));
    if (!this.token) throw new Error('board:no-token');
    for (const roomId of opts.rooms) this.watch(roomId);
  }

  // ── public surface (I2: the only two methods views may rely on) ──────────

  /** Watch an extra room (opens its signaling connection). */
  watch(roomId: string): void {
    if (this.conns.has(roomId)) return;
    const conn: RoomConn = {
      roomId,
      state: 'connecting',
      ws: null,
      floor: new FloorClient({
        roomId,
        send: (cmd: FloorCommand) => {
          const c = this.conns.get(roomId);
          if (c?.ws && c.ws.readyState === WebSocket.OPEN) c.ws.send(JSON.stringify(cmd));
        },
        onEvent: (ev) => {
          const c = this.conns.get(roomId);
          if (!c) return;
          c.lastEvent = ev;
          this.emit();
        },
      }),
      peerId: null,
      lastEvent: null,
      attempt: 0,
      closedByUser: false,
      reconnectTimer: null,
    };
    this.conns.set(roomId, conn);
    this.open(conn);
    this.emit();
  }

  /** Send a typed floor command to one room. Throws when that room is not connected. */
  sendCommand(roomId: string, cmd: FloorCommand): void {
    const conn = this.conns.get(roomId);
    if (!conn || conn.state !== 'connected' || !conn.ws || conn.ws.readyState !== WebSocket.OPEN) {
      throw new Error(`board: room "${roomId}" is not connected`);
    }
    conn.ws.send(JSON.stringify(cmd));
  }

  /** Subscribe to snapshots; fires immediately with the current one. Returns unsubscribe. */
  subscribe(listener: (s: BoardSnapshot) => void): () => void {
    this.listeners.add(listener);
    listener(this.snapshot());
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Close every room connection (no further reconnects). */
  close(): void {
    for (const conn of this.conns.values()) {
      conn.closedByUser = true;
      if (conn.reconnectTimer) clearTimeout(conn.reconnectTimer);
      conn.state = 'closed';
      conn.ws?.close();
    }
    this.emit();
  }

  snapshot(): BoardSnapshot {
    // cached between emits — useSyncExternalStore requires a stable reference
    if (!this.cached) this.cached = this.build();
    return this.cached;
  }

  private build(): BoardSnapshot {
    const rooms: Record<string, BoardRoomSnapshot> = {};
    for (const conn of this.conns.values()) {
      rooms[conn.roomId] = {
        roomId: conn.roomId,
        conn: conn.state,
        stale: conn.state !== 'connected',
        lastEvent: conn.lastEvent,
        peerId: conn.peerId,
      };
    }
    const roster: Record<string, Record<string, WirePeerPresence>> = {};
    for (const [roomId, map] of this.rosters) {
      roster[roomId] = Object.fromEntries(map);
    }
    return { rooms, roster, version: this.version };
  }

  // ── internals ────────────────────────────────────────────────────────────

  private open(conn: RoomConn): void {
    const base = this.baseUrl.replace(/\/+$/, '');
    const wsUrl = `${base.replace(/^http/, 'ws')}/ws`;
    const ws = this.wsFactory(wsUrl, [this.token as string]);
    conn.ws = ws;
    conn.state = 'connecting';

    ws.onopen = () => {
      // peer_role is the media role; floor eligibility is account-based server-side.
      ws.send(JSON.stringify({ type: 'room_join', room_id: conn.roomId, peer_role: 'consumer', protocol: 4 }));
    };

    ws.onmessage = (e: MessageEvent) => {
      const data = typeof e.data === 'string' ? e.data : '';
      if (conn.floor.handleWire(data)) {
        this.emit(); // floor frame consumed (already stale-filtered)
        return;
      }
      try {
        const msg = JSON.parse(data) as {
          type?: string;
          peer_id?: unknown;
          room_id?: unknown;
          peers?: unknown;
          device_id?: unknown;
        };
        if (msg.type === 'room_joined' && typeof msg.peer_id === 'string') {
          conn.peerId = msg.peer_id;
          conn.state = 'connected';
          conn.attempt = 0;
          this.emit();
          return;
        }
        // SB-3 roster events (T6): join-time snapshot / incremental presence.
        if (typeof msg.room_id === 'string') {
          const room = this.rosters.get(msg.room_id) ?? new Map<string, WirePeerPresence>();
          this.rosters.set(msg.room_id, room);
          if (msg.type === 'room_peers' && Array.isArray(msg.peers)) {
            room.clear();
            for (const p of msg.peers as WirePeerPresence[]) {
              if (p && typeof p.peer_id === 'string') room.set(p.peer_id, p);
            }
            this.emit();
          } else if (msg.type === 'peer_joined' && typeof msg.peer_id === 'string') {
            // presence-only by contract; identity comes from room_peers
            const prev = room.get(msg.peer_id);
            room.set(msg.peer_id, {
              peer_id: msg.peer_id,
              username: prev?.username,
              role: prev?.role,
              device_id: typeof msg.device_id === 'string' ? msg.device_id : prev?.device_id,
            });
            this.emit();
          } else if (msg.type === 'room_leave' && typeof msg.peer_id === 'string') {
            room.delete(msg.peer_id);
            this.emit();
          }
        }
      } catch {
        // non-JSON frame — ignore (board is signaling-only)
      }
    };

    ws.onclose = () => {
      if (conn.closedByUser) return;
      conn.state = 'reconnecting';
      conn.peerId = null;
      // discipline: a down connection is unknown, never idle — stale flag flips here
      this.emit();
      const delay = Math.min(this.reconnectBaseMs * 2 ** conn.attempt, MAX_BACKOFF_MS);
      conn.attempt += 1;
      conn.reconnectTimer = setTimeout(() => {
        conn.reconnectTimer = null;
        if (!conn.closedByUser) this.open(conn);
      }, delay);
    };

    ws.onerror = () => {
      /* onclose always follows; nothing to do here */
    };
  }

  private emit(): void {
    this.version += 1;
    this.cached = this.build();
    for (const l of this.listeners) l(this.cached);
  }
}
