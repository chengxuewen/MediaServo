import type { SfuStreamStatRow, AuditEventEntry } from './api/client';

// V8: an EXPLICIT, clearly-labeled simulated preview (mirrors the prototype's own
// "Simulated video — prototype feeds, not real streams" note). Opt-in via ?demo=1 or
// the Demo toggle; NEVER presented as real. Used so the visual-parity harness can
// compare like-for-like against the simulated prototype. Real mode stays honest.
export function isDemo(): boolean {
  try {
    return new URLSearchParams(window.location.search).has('demo') || localStorage.getItem('ms_demo') === '1';
  } catch { return false; }
}

// V44: room = DEVICE name, label = CAMERA/stream name (what the real Host sends with
// Produce.label). The tile title is "<device> · <camera>", as in the prototype.
const row = (id: string, room: string, label: string, score: number, bit: number | null): SfuStreamStatRow => ({
  room, label, owner: null, peer_id: 'demo-host', role: 'producer', id, kind: 'video',
  byte_count: 1e6, packet_count: 1e4, score, transport_id: `t-${id}`, local_port: 20000,
  remote_ip: '127.0.0.1', remote_port: 40000, rtp_packet_loss_sent: 0, rtp_packet_loss_received: 0,
  available_outgoing_bitrate: null, available_incoming_bitrate: bit,
});

export const DEMO_PRODUCERS: SfuStreamStatRow[] = [
  row('d1', 'VH-07', 'front', 10, 2100000),
  row('d2', 'VH-07', 'rear', 10, 2100000),
  row('d3', 'VH-07', 'cabin', 10, 900000),
  row('d4', 'VH-11', 'front', 10, 2100000),
  row('d5', 'VH-11', 'rear', 6, 600000), // degraded
];
// d6 is on the wall but absent from producers -> SIGNAL LOST tile (prototype parity).
export const DEMO_PLAYING = ['d1', 'd2', 'd3', 'd4', 'd5', 'd6'];

const now = Math.floor(Date.now() / 1000);
export const DEMO_EVENTS: AuditEventEntry[] = [
  { kind: 'device_offline', ts: now - 60, room_id: 'VH-02 · front' },
  { kind: 'auth_failure', ts: now - 120, username: 'intruder' },
  { kind: 'estop_audit', ts: now - 300, room_id: 'VH-11 · rear' },
  { kind: 'device_online', ts: now - 600, room_id: 'VH-07 · front' },
];
// Simulated recording blocks for the playback track (fractions of a 24h window).
export const DEMO_BLOCKS: Array<[number, number]> = [[0.05, 0.12], [0.2, 0.08], [0.42, 0.1], [0.66, 0.06], [0.8, 0.09]];