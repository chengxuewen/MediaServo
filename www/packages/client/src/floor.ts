// Floor client (P3-1, O29-cleared): typed commands + typed server events with
// stale-generation filtering. Transport-agnostic — the SFU ws delegates wire
// frames here via handleWire() and sends commands through the injected `send`.
//
// Wire contract (dialect 4, mirrors mediaservo-common protocol.rs):
// - C→S floor commands carry NO identity fields (C1: the server derives
//   room/peer from the connection; forged fields would be ignored anyway).
// - generation is opaque and monotonic within a server lifetime; O29 fixed the
//   seed to an in-process counter, so values stay far below 2^53 and plain
//   `Number` compares them exactly (no BigInt/string needed). Discard any
//   event whose generation is OLDER than the newest one seen for the room.
// - `floor_state` is the late-joiner replay: its snapshot carries the grant
//   truth (broadcasts never replay) — feed it through handleWire() as well.

export type FloorDenyReason =
  | 'busy'
  | 'exceeds_ceiling'
  | 'not_member'
  | 'rate_limited'
  | 'preempt_priority'
  | 'no_media'
  | 'unauthorized';

export interface FloorGranted {
  type: 'floor_granted';
  room_id: string;
  grants: string[];
  generation: number;
}
export interface FloorTaken {
  type: 'floor_taken';
  room_id: string;
  by: string;
  generation: number;
}
export interface FloorDenied {
  type: 'floor_denied';
  room_id: string;
  reason: FloorDenyReason;
  generation: number;
}
export interface FloorQueued {
  type: 'floor_queued';
  room_id: string;
  position: number;
  generation: number;
}
export interface FloorIdle {
  type: 'floor_idle';
  room_id: string;
  reason?: string;
  generation: number;
}
export interface FloorQueueEntry {
  peer: string;
  priority: number;
}
export interface FloorStateMsg {
  type: 'floor_state';
  room_id: string;
  snapshot: {
    mode: string;
    grants: string[];
    muted: string[];
    queue?: FloorQueueEntry[];
  };
  generation: number;
}

export type FloorServerEvent =
  | FloorGranted
  | FloorTaken
  | FloorDenied
  | FloorQueued
  | FloorIdle
  | FloorStateMsg;

export type FloorCommand =
  | { type: 'floor_request'; priority?: number; preempt?: boolean }
  | { type: 'floor_release' }
  | { type: 'floor_mode_set'; mode: string }
  | { type: 'floor_mute_set'; target_peer_id: string; muted: boolean }
  | { type: 'floor_set_priority'; target_peer_id: string; priority: number }
  | { type: 'floor_resync' };

/** O29 guard: generation values are plain-Number-safe by construction. */
export function generationIsTsSafe(generation: number): boolean {
  return Number.isSafeInteger(generation);
}

/** True when `generation` is older than the newest seen (stale — discard). */
export function isStaleGeneration(generation: number, newestSeen: number): boolean {
  return generation < newestSeen;
}

export interface FloorClientOptions {
  /** Transport injection: typically `(cmd) => ws.send(JSON.stringify(cmd))`. */
  send: (cmd: FloorCommand) => void;
  roomId: string;
  /** Typed, stale-filtered server events. */
  onEvent: (ev: FloorServerEvent) => void;
}

export class FloorClient {
  private newestGeneration = Number.NEGATIVE_INFINITY;
  private readonly grantPeers = new Set<string>();

  constructor(private readonly opts: FloorClientOptions) {}

  /** Latest generation seen for the room (undefined before any floor frame). */
  get generation(): number | undefined {
    return this.newestGeneration === Number.NEGATIVE_INFINITY ? undefined : this.newestGeneration;
  }

  /** Peers currently granted (from the most recent grant/idle/state truth). */
  get grants(): string[] {
    return [...this.grantPeers];
  }

  request(priority?: number, preempt?: boolean): void {
    const cmd: FloorCommand = { type: 'floor_request' };
    if (priority !== undefined) (cmd as { priority?: number }).priority = priority;
    if (preempt !== undefined) (cmd as { preempt?: boolean }).preempt = preempt;
    this.opts.send(cmd);
  }

  release(): void {
    this.opts.send({ type: 'floor_release' });
  }

  resync(): void {
    this.opts.send({ type: 'floor_resync' });
  }

  modeSet(mode: string): void {
    this.opts.send({ type: 'floor_mode_set', mode });
  }

  muteSet(targetPeerId: string, muted: boolean): void {
    this.opts.send({ type: 'floor_mute_set', target_peer_id: targetPeerId, muted });
  }

  /** O18: dispatcher adjusts a queued peer's request priority (server reorders
   *  priority-first and answers with a private position update + snapshot). */
  setPriority(targetPeerId: string, priority: number): void {
    this.opts.send({ type: 'floor_set_priority', target_peer_id: targetPeerId, priority });
  }

  /**
   * Feed one raw wire frame. Returns true when the frame was a floor message
   * (consumed here — the SFU handleMessage must not process it further).
   */
  handleWire(data: string): boolean {
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(data) as Record<string, unknown>;
    } catch {
      return false;
    }
    const type = msg.type;
    if (typeof type !== 'string' || !type.startsWith('floor_')) return false;

    if (type === 'floor_state') {
      const st = msg as unknown as FloorStateMsg;
      if (isStaleGeneration(st.generation, this.newestGeneration)) return true;
      this.newestGeneration = st.generation;
      this.grantPeers.clear();
      for (const g of st.snapshot.grants) this.grantPeers.add(g);
      this.opts.onEvent(st);
      return true;
    }

    const generation = msg.generation;
    if (typeof generation !== 'number' || isStaleGeneration(generation, this.newestGeneration)) {
      return true; // floor-family frame, but stale — swallowed silently
    }
    this.newestGeneration = generation;
    if (type === 'floor_granted') {
      this.grantPeers.clear();
      for (const g of (msg as unknown as FloorGranted).grants) this.grantPeers.add(g);
    } else if (type === 'floor_taken') {
      this.grantPeers.delete((msg as unknown as FloorTaken).by);
    } else if (type === 'floor_idle') {
      this.grantPeers.clear();
    }

    this.opts.onEvent(msg as unknown as FloorServerEvent);
    return true;
  }
}
