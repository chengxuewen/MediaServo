import { useCallback, useEffect, useState } from 'react';
import { getVehicleStatus, type StatusReport } from '../api/client';
import './Vehicles.css';
import { Car } from 'lucide-react';

function fmtUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
}

function fmtBps(bps: number): string {
  if (bps >= 1024 * 1024) return `${(bps / 1024 / 1024).toFixed(1)} MB/s`;
  return `${(bps / 1024).toFixed(0)} KB/s`;
}

export default function Vehicles() {
  const [vehicles, setVehicles] = useState<{ room_id: string; report: StatusReport }[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    try {
      const data = await getVehicleStatus();
      setVehicles(data.vehicles);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch vehicle status');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchStatus(); }, [fetchStatus]);
  // ponytail: 5s 轮询（与 useDevices 同节奏）
  useEffect(() => {
    const interval = setInterval(fetchStatus, 5000);
    return () => clearInterval(interval);
  }, [fetchStatus]);

  if (loading) return <div className="loading">Loading...</div>;
  if (error && vehicles.length === 0) return <div className="error">{error}</div>;

  return (
    <div className="vehicles">
      <div className="vehicles-head">
        <h2 className="section-title">Vehicle Status</h2>
        <span className="vehicles-count">{vehicles.length} vehicles reporting</span>
      </div>
      {vehicles.length === 0 ? (
        <p className="empty">No vehicle status reports</p>
      ) : (
        <div className="vehicle-list">
          {vehicles.map(({ room_id, report }) => (
            <div key={room_id} className="vehicle-card">
              <div className="vehicle-header">
                <span className="vehicle-name"><Car size={14} /> {room_id}</span>
                <span className={`vehicle-signal ${report.signal.remote_connected ? 'ok' : 'down'}`}>
                  <span className="status-dot" />
                  {report.signal.remote_connected ? 'signal ok' : 'signal lost'}
                </span>
                <span className="vehicle-meta">
                  {fmtUptime(report.signal.agent_uptime_secs)} · {report.processes.length} procs · {report.topics.length} topics · {report.streams.length} streams
                </span>
              </div>

              <div className="vehicle-body">
                <div className="vehicle-col">
                  <h4>Processes</h4>
                  {report.processes.length === 0 ? <p className="vehicle-empty">none</p> : (
                    <ul className="vehicle-list-ul">
                      {report.processes.map((p) => (
                        <li key={p.name} className="process-row">
                          <span>{p.name}</span>
                          <span className={p.running ? 'proc-running' : 'proc-down'}>{p.running ? 'running' : p.expected ? 'down' : 'unexpected'}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>

                <div className="vehicle-col">
                  <h4>Topics</h4>
                  {report.topics.length === 0 ? <p className="vehicle-empty">none</p> : (
                    <ul className="vehicle-list-ul">
                      {report.topics.map((t) => (
                        <li key={t.topic} className="topic-row">
                          <span className="topic-name">{t.topic}</span>
                          <span>{t.fps.toFixed(1)} fps</span>
                          <span>{fmtBps(t.bps)}</span>
                          <span className={t.stalled ? 'topic-stalled' : 'topic-ok'}>{t.stalled ? 'stalled' : 'ok'}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>

                <div className="vehicle-col">
                  <h4>Streams</h4>
                  {report.streams.length === 0 ? <p className="vehicle-empty">none</p> : (
                    <ul className="vehicle-list-ul">
                      {report.streams.map((s) => (
                        <li key={s.id} className="stream-row">
                          <span>{s.id}</span>
                          <span>{s.frame_width}×{s.frame_height}</span>
                          <span>{s.frames_encoded} frames</span>
                          <span className={s.connected ? 'proc-running' : 'proc-down'}>{s.connected ? 'connected' : 'disconnected'}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
