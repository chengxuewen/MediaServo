import { useEffect, useRef, useState } from 'react';
import type { AuditEventEntry, SfuStreamStatRow } from '../api/client';
import './AlertPanel.css';

// B4 (F5): alert-worthy kinds per the adjudicated restyle:256 Alerts口径 (full names).
const DANGER = new Set(['device_offline', 'estop_audit', 'emergency_command']);
const WARN = new Set(['auth_failure', 'authorization_denied', 'stream_destroy', 'device_revoked']);
const ALERT_KINDS = new Set<string>([...DANGER, ...WARN]);
const key = (e: AuditEventEntry) => `${e.ts}-${e.kind}-${e.room_id ?? e.device_id ?? e.username ?? ''}`;

/**
 * Right-rail alert panel (B4, monitor-prototype-parity): the alert-worthy subset of
 * the audit ring (danger/warn per restyle:256), with filter chips, an unread badge,
 * click-to-wall (plays the alert's device room), mark-all-read, and an auto-wall arm
 * toggle. Auto-wall plays a NEW danger alert's room; the cap16 skip + hint is handled
 * by the wall's addPlaying (O5). Data source mirrors Timeline (M6 single-WS merge in B6).
 */
export default function AlertPanel({ events, producers, onPlayRoom }: {
  events: AuditEventEntry[];
  producers: SfuStreamStatRow[];
  onPlayRoom: (room: string) => void;
}) {
  const [filter, setFilter] = useState<'all' | 'danger' | 'warn'>('all');
  const [read, setRead] = useState<Set<string>>(new Set());
  const [autoWall, setAutoWall] = useState(false);
  const seenRef = useRef<Set<string>>(new Set());

  const alerts = events.filter((e) => ALERT_KINDS.has(e.kind));
  const unread = alerts.filter((e) => !read.has(key(e))).length;
  const filtered = alerts.filter((e) => (filter === 'all' ? true : filter === 'danger' ? DANGER.has(e.kind) : WARN.has(e.kind)));

  // Auto-wall: play a NEW danger alert's room while armed. Mark seen only AFTER a
  // play decision so a producer not yet loaded (audit 2s poll outruns stats 5s) is
  // retried on the next producers update (B1 review fix: seen-before-check race).
  // device_offline has no live stream to wall (its producer is gone) — honest skip;
  // the alert stays visible in the panel. addPlaying enforces cap16 (full → skip, O5).
  useEffect(() => {
    for (const e of alerts) {
      const k = key(e);
      if (seenRef.current.has(k)) continue;
      if (!autoWall) { seenRef.current.add(k); continue; }
      const room = typeof e.room_id === 'string' ? e.room_id : null;
      if (e.kind === 'device_offline') { seenRef.current.add(k); continue; }
      if (room && producers.some((p) => p.room === room)) {
        seenRef.current.add(k);
        onPlayRoom(room);
      }
      // else: playable danger, producer not loaded yet — leave unseen, retry next update
    }
  }, [alerts, autoWall, producers, onPlayRoom]);

  const click = (e: AuditEventEntry) => {
    setRead((prev) => new Set(prev).add(key(e)));
    if (typeof e.room_id === 'string') onPlayRoom(e.room_id);
  };

  return (
    <div className="alert-panel">
      <div className="ap-head">
        <span className="ap-unread" role="status" aria-label={`${unread} unread alerts`}>{unread}</span>
        <button className="ap-markall" onClick={() => setRead(new Set(alerts.map(key)))}>Mark all read</button>
        <button className={`ap-autowall${autoWall ? ' on' : ''}`} aria-pressed={autoWall} onClick={() => setAutoWall((a) => !a)}>Auto-wall</button>
      </div>
      <div className="ap-filter" role="group" aria-label="Filter alerts">
        {(['all', 'danger', 'warn'] as const).map((f) => (
          <button key={f} className={`chip${filter === f ? ' on' : ''}`} aria-pressed={filter === f} onClick={() => setFilter(f)}>{f}</button>
        ))}
      </div>
      <div className="ap-list">
        {filtered.length === 0 && <div className="ap-empty">No {filter} alerts</div>}
        {filtered.map((e) => (
          <div
            key={key(e)}
            role="button"
            tabIndex={0}
            className={`ap-row${read.has(key(e)) ? ' read' : ''}`}
            onClick={() => click(e)}
            onKeyDown={(ev) => { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); click(e); } }}
          >
            <span className={`ap-tag ${DANGER.has(e.kind) ? 'danger' : 'warn'}`}>{e.kind}</span>
            <span className="ap-room">{String(e.room_id ?? e.device_id ?? e.username ?? '')}</span>
          </div>
        ))}
      </div>
    </div>
  );
}