import { useCallback, useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { useNavigate } from 'react-router-dom';
import { useDevices } from '../hooks/useDevices';
import { useAdminWS } from '../hooks/useAdminWS';
import { useAuth } from '../hooks/useAuth';
import {
  getAdminDevices, getPendingDevices, listManagedRooms, getVehicleStatus, getAuditEvents,
} from '../api/client';
import type { AuditEventEntry, StatusReport } from '../api/client';
import VideoPlayer from '../components/VideoPlayer';
import './Dashboard.css';
import { RefreshCw } from 'lucide-react';

/**
 * Overview (admin-prototype-restyle W1): fleet health at a glance — the
 * prototype shape, not a resource counter. Six health stats + Fleet summary +
 * Alerts (last 24 h, audit ring) + Quick check (≤4 live tiles). The device
 * long-list/playback wall relocated to Admin → Devices (ActiveDevices).
 *
 * Health derivation (real data, no new backend):
 * - online   = active device rooms with every stream publishing (useDevices)
 * - degraded = active device rooms with ≥1 non-publishing stream (weak link)
 * - offline  = registered devices (AdminState) absent from the active set
 * - pending  = device-enroll approval queue
 * - live rooms = managed-rooms entries (across kinds)
 * - alerts   = /api/admin/audit events within 24 h, alert-worthy kinds only
 */

// Alert-worthy audit kinds → severity (operational noise like peer/consumer
// join-leave and auth_success stays out of the alert face).
const ALERT_SEV: Record<string, 'danger' | 'warn'> = {
  device_offline: 'danger',
  estop_audit: 'danger',
  emergency_command: 'danger',
  auth_failure: 'warn',
  authorization_denied: 'warn',
  stream_destroy: 'warn',
  device_revoked: 'warn',
};

const DAY_SECS = 24 * 3600;
const ALERTS_SHOWN = 8;
const QUICK_CAP = 4;

interface Alert { sev: 'danger' | 'warn'; txt: string; time: string; ts: number; }

function field(e: AuditEventEntry, k: string): string {
  const v = e[k];
  return v === undefined || v === null ? '—' : String(v);
}

function alertText(e: AuditEventEntry): string {
  switch (e.kind) {
    case 'device_offline': return `${field(e, 'device_id')} signal lost`;
    case 'device_revoked': return `${field(e, 'device_id')} revoked by ${field(e, 'actor')}`;
    case 'auth_failure': return `Auth failure: ${field(e, 'peer_id')} — ${field(e, 'reason')}`;
    case 'authorization_denied': return `Denied ${field(e, 'action')}: ${field(e, 'peer_id')}`;
    case 'stream_destroy': return `Stream ${field(e, 'stream_id')} destroyed`;
    case 'estop_audit': return `E-stop ${field(e, 'cmd')} in ${field(e, 'room_id')}`;
    case 'emergency_command': return `Emergency ${field(e, 'command')} → ${field(e, 'vehicle')} by ${field(e, 'username')}`;
    default: return e.kind;
  }
}

function fmtTime(ts: number): string {
  const d = new Date(ts * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function toAlerts(events: AuditEventEntry[]): Alert[] {
  const cutoff = Date.now() / 1000 - DAY_SECS;
  return events
    .filter((e) => e.kind in ALERT_SEV && e.ts >= cutoff)
    .sort((a, b) => b.ts - a.ts)
    .map((e) => ({ sev: ALERT_SEV[e.kind], txt: alertText(e), time: fmtTime(e.ts), ts: e.ts }));
}

// Fleet summary (prototype fleetCounts): reporting vehicles; signal lost =
// remote link down; agent unreachable = active device without a status report;
// stalled topics / down (expected) processes summed across reports.
function fleetCounts(reports: { room_id: string; report: StatusReport }[], activeIds: Set<string>) {
  let x = 0, z = 0, w = 0;
  const reported = new Set(reports.map((r) => r.room_id));
  for (const { report } of reports) {
    if (!report.signal.remote_connected) x++;
    z += report.topics.filter((t) => t.stalled).length;
    w += report.processes.filter((p) => !p.running && p.expected).length;
  }
  const y = [...activeIds].filter((id) => !reported.has(id)).length;
  return { n: reports.length, x, y, z, w };
}

export default function Dashboard() {
  const navigate = useNavigate();
  const { isAdmin } = useAuth();
  const { devices, loading, error, refetch: refetchDevices } = useDevices();
  // aux faces: null = unknown (loading or failed) → render '—', never a fake 0
  const [registered, setRegistered] = useState<string[] | null>(null);
  const [pendingCount, setPendingCount] = useState<number | null>(null);
  const [roomCount, setRoomCount] = useState<number | null>(null);
  const [reports, setReports] = useState<{ room_id: string; report: StatusReport }[] | null>(null);
  const [alerts, setAlerts] = useState<Alert[] | null>(null);
  // quick check: dismissed tiles stay dismissed; the next live stream fills in
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());

  const fetchAux = useCallback(() => {
    getAdminDevices().then((r) => setRegistered(r.devices.map((d) => d.device_id))).catch(() => setRegistered(null));
    getPendingDevices().then((r) => setPendingCount(r.count)).catch(() => setPendingCount(null));
    listManagedRooms().then((r) => setRoomCount(r.count)).catch(() => setRoomCount(null));
    getVehicleStatus().then((r) => setReports(r.vehicles)).catch(() => setReports(null));
    getAuditEvents().then((r) => setAlerts(toAlerts(r.events))).catch(() => setAlerts(null));
  }, []);

  useEffect(() => { fetchAux(); }, [fetchAux]);
  // aux freshness: WS events + a slow poll backstop (devices already poll 5 s)
  useEffect(() => {
    const interval = setInterval(fetchAux, 30000);
    return () => clearInterval(interval);
  }, [fetchAux]);

  const onWsEvent = useCallback(() => {
    refetchDevices();
    fetchAux();
  }, [refetchDevices, fetchAux]);
  useAdminWS(onWsEvent);

  const refresh = () => { refetchDevices(); fetchAux(); };

  if (loading) return <div className="loading">Loading...</div>;
  if (error) return <div className="error">{error}</div>;

  const activeIds = new Set(devices.map((d) => d.device_id));
  const online = devices.filter((d) => d.streams.every((s) => s.online)).length;
  const degraded = devices.length - online;
  const offline = registered ? registered.filter((id) => !activeIds.has(id)).length : null;
  const total = registered ? new Set([...registered, ...activeIds]).size : null;
  const fleet = reports ? fleetCounts(reports, activeIds) : null;

  const stats: { k: string; v: string | number; d: string; c: string }[] = [
    { k: 'Devices online', v: online, d: total !== null ? `of ${total}` : '—', c: 'var(--ms-color-success)' },
    { k: 'Degraded', v: degraded, d: 'weak link', c: 'var(--ms-color-warning)' },
    { k: 'Offline', v: offline ?? '—', d: 'signal lost', c: 'var(--ms-color-danger)' },
    { k: 'Pending approval', v: pendingCount ?? '—', d: 'devices queue', c: 'var(--ms-color-primary)' },
    { k: 'Live rooms', v: roomCount ?? '—', d: 'across kinds', c: 'var(--ms-color-primary)' },
    { k: 'Active alerts', v: alerts ? alerts.length : '—', d: 'last 24 h', c: 'var(--ms-color-warning)' },
  ];

  const quick = devices
    .flatMap((d) => d.streams.filter((s) => s.online).map((s) => ({ key: `${d.device_id}_${s.stream_id}`, deviceId: d.device_id, streamId: s.stream_id })))
    .filter((q) => !dismissed.has(q.key))
    .slice(0, QUICK_CAP);

  return (
    <div className="dashboard">
      <div className="ov-head">
        <div>
          <h1>Overview</h1>
          <p className="ov-sub">Fleet health at a glance</p>
        </div>
        <button className="ov-btn" onClick={refresh}><RefreshCw size={12} /> Refresh</button>
      </div>

      <div className="ov-stats">
        {stats.map((s) => (
          <div className="ov-stat" key={s.k} style={{ '--sc': s.c } as CSSProperties}>
            <span className="ov-stat-k">{s.k}</span>
            <span className="ov-stat-v">{s.v}</span>
            <span className="ov-stat-d">{s.d}</span>
          </div>
        ))}
      </div>

      <div className="ov-card ov-fleet">
        <div className="ov-card-title">Fleet</div>
        <p className="ov-fleet-summary">
          {fleet
            ? `${fleet.n} vehicles · ${fleet.x} signal lost · ${fleet.y} agent unreachable · ${fleet.z} stalled topics · ${fleet.w} down processes`
            : '—'}
        </p>
        <button className="ov-btn" onClick={() => navigate('/admin', { state: { tab: 'fleet' } })}>Open fleet lens</button>
      </div>

      <div className="ov-grid">
        <div className="ov-card">
          <div className="ov-card-title">Alerts · last 24 h</div>
          {alerts === null ? (
            <p className="ov-empty">—</p>
          ) : alerts.length === 0 ? (
            <p className="ov-empty">No alerts in the last 24 h</p>
          ) : (
            <div className="ov-alerts">
              {alerts.slice(0, ALERTS_SHOWN).map((a, i) => (
                <div key={`${a.ts}-${i}`} className={`ov-alert ${a.sev}`}>
                  <span className="ov-alert-dot" aria-hidden="true" />
                  <span className="ov-alert-txt">{a.txt}</span>
                  <span className="ov-alert-time">{a.time}</span>
                </div>
              ))}
              {alerts.length > ALERTS_SHOWN && (
                <p className="ov-more">+{alerts.length - ALERTS_SHOWN} more in the last 24 h</p>
              )}
            </div>
          )}
        </div>

        <div className="ov-card">
          <div className="ov-card-title">Quick check <span className="ov-chip">≤4 tiles</span></div>
          {quick.length === 0 ? (
            <p className="ov-empty">No live streams</p>
          ) : (
            <div className="ov-quick">
              {quick.map((q) => (
                <VideoPlayer
                  key={q.key}
                  roomId={q.deviceId}
                  streamLabel={q.streamId}
                  serverUrl={wsServerUrl()}
                  token={localStorage.getItem('mediaservo_admin_token') || ''}
                  variant="tile"
                  onClose={() => setDismissed((prev) => new Set(prev).add(q.key))}
                />
              ))}
            </div>
          )}
          <p className="ov-note">
            Full device wall lives in{' '}
            {isAdmin ? (
              <button className="ov-link" onClick={() => navigate('/admin', { state: { tab: 'devices' } })}>Admin → Devices</button>
            ) : (
              <span>Admin → Devices (admin access)</span>
            )}.
          </p>
        </div>
      </div>
    </div>
  );
}

function wsServerUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}