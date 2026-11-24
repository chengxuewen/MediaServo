import { useCallback } from 'react';
import { StatusGlyph, type GlyphTone } from '@mediaservo/react';
import type { AuditEventEntry } from '../api/client';
import { useAudit } from '../hooks/useAudit';
import { useAdminWS } from '../hooks/useAdminWS';
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
 * Realtime event timeline (P3 slice ③c): the audit ring (/api/admin/audit) polled
 * 2s + refetched on admin WS events (/api/admin/events). Entries grow as audited
 * actions land. Honest boundary: core-in-memory ring cap 256, not persisted.
 */
export default function Timeline() {
  const { events, loading, error, refetch } = useAudit();
  const onWs = useCallback(() => { refetch(); }, [refetch]);
  useAdminWS(onWs);

  return (
    <section className="timeline" aria-label="Realtime event timeline">
      <h2 className="tl-title">Realtime timeline <span className="tl-count">{events.length} events</span></h2>
      {loading ? (
        <p className="tl-state">Loading events…</p>
      ) : error ? (
        <p className="tl-state tl-error">{error}</p>
      ) : events.length === 0 ? (
        <p className="tl-state">No events yet.</p>
      ) : (
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
      )}
    </section>
  );
}