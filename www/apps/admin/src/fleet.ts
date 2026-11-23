// Fleet health counts (B4 approved real face, adjudication 4): single source
// for the overview fleet card AND the Devices fleet lens strip.
// Supersede rule (prototype fleetCounts): an unreachable agent (no report, or
// report older than STALE_SECS) counts ONLY as unreachable — its topic/process
// bad-counts are NOT added again (fixes the W1 double-count).
import type { StatusReport } from './api/client';

export const STALE_SECS = 30; // B4 micro-decision default (board open item)

export interface FleetCounts { n: number; x: number; y: number; z: number; w: number; }

export function fleetCounts(
  reports: { room_id: string; report: StatusReport }[],
  activeIds: Set<string>,
  nowSecs: number = Date.now() / 1000,
): FleetCounts {
  let x = 0, y = 0, z = 0, w = 0;
  const reported = new Set(reports.map((r) => r.room_id));
  const stale = new Set<string>();
  for (const { room_id, report } of reports) {
    if (nowSecs - report.ts > STALE_SECS) { stale.add(room_id); continue; }
    if (!report.signal.remote_connected) x++;
    z += report.topics.filter((t) => t.stalled).length;
    w += report.processes.filter((p) => !p.running && p.expected).length;
  }
  // unreachable = active device without any report + stale-report rooms
  y = [...activeIds].filter((id) => !reported.has(id)).length + stale.size;
  return { n: reports.length, x, y, z, w };
}

/** Per-device telemetry summary for a fresh report (null when stale/absent). */
export function telemetrySummary(
  reports: { room_id: string; report: StatusReport }[],
  deviceId: string,
  nowSecs: number = Date.now() / 1000,
): { signal: 'ok' | 'lost'; stalled: number; down: number } | null {
  const hit = reports.find((r) => r.room_id === deviceId);
  if (!hit || nowSecs - hit.report.ts > STALE_SECS) return null;
  return {
    signal: hit.report.signal.remote_connected ? 'ok' : 'lost',
    stalled: hit.report.topics.filter((t) => t.stalled).length,
    down: hit.report.processes.filter((p) => !p.running && p.expected).length,
  };
}