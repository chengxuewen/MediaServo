import { useEffect, useRef, useState } from 'react';
import type { AuditEventEntry, SfuStreamStatRow } from '../api/client';
import './AlertPanel.css';

// B4/K11=A: alert-worthy subset of the audit ring. Categories are mapped HONESTLY to the
// prototype's severity words — danger -> "Alarm", warn -> "Warn". The prototype's Motion/
// Bookmark categories have no backend source, so they are deferred (not faked).
const DANGER = new Set(['device_offline', 'estop_audit', 'emergency_command']);
const WARN = new Set(['auth_failure', 'authorization_denied', 'stream_destroy', 'device_revoked']);
const ALERT_KINDS = new Set<string>([...DANGER, ...WARN]);
const TITLE: Record<string, string> = {
  device_offline: 'Device offline', estop_audit: 'E-stop engaged', emergency_command: 'Emergency command',
  auth_failure: 'Authentication failure', authorization_denied: 'Access denied',
  stream_destroy: 'Stream ended', device_revoked: 'Device revoked',
};
const sevWord = (k: string) => (DANGER.has(k) ? 'Alarm' : 'Warn');
const key = (e: AuditEventEntry) => `${e.ts}-${e.kind}-${e.room_id ?? e.device_id ?? e.username ?? ''}`;
const fmtTime = (ts: number) => new Date(ts * 1000).toLocaleTimeString('en-GB', { hour12: false });

/**
 * Right-rail alert panel (B4 + V9c/K11=A): prototype row language — severity dot + human
 * title + mono time + severity word tag (Alarm/Warn). Filter chips All/Alarm/Warn (Motion
 * & Bookmark deferred — no data source). Click focuses/zooms the alert's camera (adds it
 * to the wall if absent). Auto-wall plays a NEW danger alert's room (cap16 skip via wall).
 */
export default function AlertPanel({ events, producers, onPlayRoom, onFocusRoom }: {
  events: AuditEventEntry[];
  producers: SfuStreamStatRow[];
  onPlayRoom: (room: string) => void;
  onFocusRoom: (room: string) => void;
}) {
  const [filter, setFilter] = useState<'all' | 'alarm' | 'warn'>('all');
  const [read, setRead] = useState<Set<string>>(new Set());
  const [autoWall, setAutoWall] = useState(false);
  const seenRef = useRef<Set<string>>(new Set());

  const alerts = events.filter((e) => ALERT_KINDS.has(e.kind));
  const unread = alerts.filter((e) => !read.has(key(e))).length;
  const filtered = alerts.filter((e) => (filter === 'all' ? true : filter === 'alarm' ? DANGER.has(e.kind) : WARN.has(e.kind)));

  // Auto-wall: play a NEW danger alert's room while armed; mark seen only AFTER a play
  // decision (audit 2s poll outruns stats 5s, so retry when the producer loads). device_
  // offline has no live stream to wall -> honest skip. addPlaying enforces cap16 (O5).
  useEffect(() => {
    for (const e of alerts) {
      const k = key(e);
      if (seenRef.current.has(k)) continue;
      if (!autoWall) { seenRef.current.add(k); continue; }
      const room = typeof e.room_id === 'string' ? e.room_id : null;
      if (e.kind === 'device_offline') { seenRef.current.add(k); continue; }
      if (room && producers.some((p) => p.room === room)) { seenRef.current.add(k); onPlayRoom(room); }
    }
  }, [alerts, autoWall, producers, onPlayRoom]);

  const click = (e: AuditEventEntry) => {
    setRead((prev) => new Set(prev).add(key(e)));
    if (typeof e.room_id === 'string') onFocusRoom(e.room_id);
  };

  return (
    <div className="alert-panel">
      <div className="ap-head">
        <span className="ap-unread" role="status" aria-label={`${unread} unread alerts`}>{unread}</span>
        <button className="ap-markall" onClick={() => setRead(new Set(alerts.map(key)))}>Mark all read</button>
        <button className={`ap-autowall${autoWall ? ' on' : ''}`} aria-pressed={autoWall} onClick={() => setAutoWall((a) => !a)}>Auto-wall</button>
      </div>
      <div className="ap-filter" role="group" aria-label="Filter alerts">
        {(['all', 'alarm', 'warn'] as const).map((f) => (
          <button key={f} className={`chip${filter === f ? ' on' : ''}`} aria-pressed={filter === f} onClick={() => setFilter(f)}>{f === 'all' ? 'All' : f === 'alarm' ? 'Alarm' : 'Warn'}</button>
        ))}
      </div>
      <div className="ap-list">
        {filtered.length === 0 && <div className="ap-empty">No {filter === 'all' ? '' : filter} alerts</div>}
        {filtered.map((e) => {
          const danger = DANGER.has(e.kind);
          return (
            <div key={key(e)} role="button" tabIndex={0}
              className={`ap-row${read.has(key(e)) ? ' read' : ''}`}
              onClick={() => click(e)}
              onKeyDown={(ev) => { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); click(e); } }}>
              <span className={`ap-dot ${danger ? 'danger' : 'warn'}`} aria-hidden="true" />
              <span className="ap-title">{TITLE[e.kind] ?? e.kind}</span>
              <span className="ap-time">{fmtTime(e.ts)}</span>
              <span className={`ap-tag ${danger ? 'danger' : 'warn'}`}>{sevWord(e.kind)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}