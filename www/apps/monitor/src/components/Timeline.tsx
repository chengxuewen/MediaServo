import { StatusGlyph, type GlyphTone } from '@mediaservo/react';
import type { AuditEventEntry } from '../api/client';
import './Timeline.css';

function fmtTime(ts: number): string {
  const d = new Date(ts * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function field(e: AuditEventEntry, k: string): string | null {
  const v = e[k];
  return v === undefined || v === null ? null : String(v);
}

// Generic summary from the common per-variant fields (audit.rs AuditEvent).
function summary(e: AuditEventEntry): string {
  const parts: string[] = [];
  for (const k of ['action', 'peer_id', 'username', 'device_id', 'room_id', 'stream_id', 'key_id', 'actor', 'reason', 'command']) {
    const v = field(e, k);
    if (v) parts.push(`${k}=${v}`);
  }
  return parts.join(' · ');
}

// Alert-worthy kinds → danger; destructive/operational-warning → warn; healthy lifecycle → ok.
const DANGER = new Set(['device_offline', 'estop_audit', 'emergency_command', 'auth_failure', 'authorization_denied', 'device_revoked']);
const WARN = new Set(['stream_destroy', 'psk_rotated', 'account_deleted', 'evidence_withdraw', 'api_key_revoked']);
const OK = new Set(['auth_success', 'device_online', 'stream_create', 'room_create', 'device_registered']);
function tone(kind: string): GlyphTone {
  if (DANGER.has(kind)) return 'danger';
  if (WARN.has(kind)) return 'warn';
  if (OK.has(kind)) return 'ok';
  return 'unknown';
}

/**
 * Event feed (P3 slice ③c; M8 renamed from "Realtime timeline" to avoid colliding
 * with the prototype's playback scrubber): the audit ring (/api/admin/audit) polled
 * 2s + refetched on admin WS events (/api/admin/events). Entries grow as audited
 * actions land. Honest boundary: core-in-memory ring cap 256, not persisted.
 */
export default function Timeline({ events, loading, error, refetch }: {
  events: AuditEventEntry[]; loading: boolean; error: string | null; refetch: () => void;
}) {
  return (
    <section className="timeline" aria-label="Event feed (audit ring)">
      <h2 className="tl-title">Event feed <span className="tl-count">{events.length} events</span></h2>
      {loading ? (
        <p className="tl-state">Loading events…</p>
      ) : error && events.length === 0 ? (
        <p className="tl-state tl-error">{error} <button className="tl-retry" onClick={refetch}>Retry</button></p>
      ) : events.length === 0 ? (
        <p className="tl-state">No events yet.</p>
      ) : (
        <>
        {error && <div className="tl-stale" role="alert">Showing last known events — refresh failed. <button className="tl-retry" onClick={refetch}>Retry</button></div>}
        <ol className="tl-list">
          {events.map((e, i) => (
            <li className="tl-entry" key={`${e.ts}-${e.kind}-${i}`}>
              <span className="tl-time">{fmtTime(e.ts)}</span>
              <StatusGlyph tone={tone(e.kind)} shape="dot" label={e.kind} />
              <span className="tl-kind">{e.kind}</span>
              <span className="tl-summary">{summary(e)}</span>
            </li>
          ))}
        </ol>
        </>
      )}
    </section>
  );
}