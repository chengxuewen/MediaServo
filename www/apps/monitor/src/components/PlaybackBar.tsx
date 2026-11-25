import type { AuditEventEntry } from '../api/client';
import './PlaybackBar.css';

// V4 (L4/K7): the prototype's bottom playback-timeline CHROME, rendered honestly.
// Real seek/speed/calendar/solo/sync are deferred (RECORDING_READY=false) -> every
// transport control is guarded (aria-disabled + reason), never a fake live control.
// The track/scale/playhead are visual; event ticks derive from the REAL audit ring
// (geometry is data-driven and excluded from the parity contract, N2).
const REASON = 'Playback review deferred — recording backend not ready (RECORDING_READY=false)';
const HOURS = ['00:00', '03:00', '06:00', '09:00', '12:00', '15:00', '18:00', '21:00'];

export default function PlaybackBar({ events }: { events: AuditEventEntry[] }) {
  const now = new Date();
  const dayStart = new Date(now); dayStart.setHours(0, 0, 0, 0);
  const span = 24 * 3600 * 1000;
  const ticks = events
    .map((e) => (e.ts * 1000 - dayStart.getTime()) / span)
    .filter((f) => f >= 0 && f <= 1);
  const liveFrac = (now.getTime() - dayStart.getTime()) / span;
  const Guard = ({ label }: { label: string }) => (
    <button className="pb-ctl" aria-disabled="true" title={REASON} data-reason={REASON}>{label}</button>
  );
  return (
    <div className="ctl-bar" aria-label="Playback timeline (review deferred)">
      <div className="ctl-top">
        <span className="ctl-time mono">{now.toLocaleTimeString('en-GB', { hour12: false })}</span>
        <Guard label="1h" /><Guard label="6h" /><Guard label="24h" />
        <Guard label="📅" /><Guard label="−1×" /><Guard label="1×" />
        <Guard label="Solo" /><Guard label="Sync" />
        <button className="pb-live" aria-disabled="true" title={REASON}>● LIVE</button>
      </div>
      <div className="tl-track">
        {ticks.map((f, i) => <span key={i} className="tl-event" style={{ left: `${f * 100}%` }} />)}
        <span className="tl-head-mark" style={{ left: `${liveFrac * 100}%` }} />
      </div>
      <div className="tl-scale">{HOURS.map((h) => <span key={h}>{h}</span>)}</div>
    </div>
  );
}