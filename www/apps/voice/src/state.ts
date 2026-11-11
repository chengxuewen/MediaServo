// Voice UI state machine (P3-3) — pure reducers so vitest can pin the M4
// interaction contract without a browser. The hook feeds FloorClient events
// in and renders from the reduced state.

import type { FloorDenyReason, FloorServerEvent } from '@mediaservo/client';

export interface FloorUiState {
  generation?: number;
  mode?: string;
  grants: string[];
  muted: string[];
  /** Snapshot queue strip: [peer, priority] in arrival order. */
  queue: Array<{ peer: string; priority: number }>;
  /** Self derived from the connection peer id (undefined before connect). */
  selfPeer?: string;
  /** Deny reason of the most recent denial of THIS peer. */
  deniedReason?: FloorDenyReason;
  lastEvent?: FloorServerEvent['type'];
}

export function initialFloorUi(selfPeer?: string): FloorUiState {
  return { grants: [], muted: [], queue: [], selfPeer, deniedReason: undefined };
}

export function reduceFloorUi(s: FloorUiState, ev: FloorServerEvent): FloorUiState {
  const next: FloorUiState = { ...s, lastEvent: ev.type, deniedReason: undefined };
  switch (ev.type) {
    case 'floor_granted':
      next.generation = ev.generation;
      next.grants = [...ev.grants];
      break;
    case 'floor_taken':
      next.generation = ev.generation;
      next.grants = s.grants.filter((g) => g !== ev.by);
      break;
    case 'floor_denied':
      next.generation = ev.generation;
      next.deniedReason = ev.reason;
      break;
    case 'floor_queued':
      next.generation = ev.generation;
      break;
    case 'floor_idle':
      next.generation = ev.generation;
      next.grants = [];
      next.queue = [];
      break;
    case 'floor_state':
      next.generation = ev.generation;
      next.mode = ev.snapshot.mode;
      next.grants = [...ev.snapshot.grants];
      next.muted = [...ev.snapshot.muted];
      next.queue = [...(ev.snapshot.queue ?? [])];
      break;
  }
  return next;
}

export type CueKind = 'granted' | 'denied' | 'queued' | 'released';

/** Cue derivation (pure): which sound the UI should play for one event.
 *  Only events that concern THIS peer fire a cue; others are silent. */
export function cueFor(prev: FloorUiState, ev: FloorServerEvent): CueKind | null {
  const me = prev.selfPeer;
  if (!me) return null;
  switch (ev.type) {
    case 'floor_granted':
      return ev.grants.includes(me) ? 'granted' : null;
    case 'floor_denied':
      return 'denied';
    case 'floor_queued':
      return 'queued';
    case 'floor_idle':
    case 'floor_taken':
      return prev.grants.includes(me) ? 'released' : null;
    default:
      return null;
  }
}

/** Hold-to-talk posture (pure): what the big button should show. */
export type TalkPosture = 'idle' | 'granted' | 'queued' | 'denied';
export function talkPosture(s: FloorUiState): TalkPosture {
  if (s.selfPeer && s.grants.includes(s.selfPeer)) return 'granted';
  if (s.deniedReason) return 'denied';
  return 'idle';
}
