import { useEffect, useState } from 'react';
import { NavLink, Outlet, useNavigate, useLocation } from 'react-router-dom';
import { useAuth } from '../hooks/useAuth';
import { clearToken } from '../api/client';
import { FloorNotifChips, useFloorNotifs } from '../notifications';
import './Layout.css';
import { Radio, LayoutDashboard, MonitorCog, Users, Settings as SettingsIcon, Sliders, FileAudio } from 'lucide-react';

export default function Layout() {
  const { role, username, canMonitor, token, isAdmin } = useAuth();
  const { heldCount } = useFloorNotifs();
  const navigate = useNavigate();
  const location = useLocation();
  const notices = (location.state as { notice?: string } | null)?.notice;
  // T8 weaknet panel entry card: probe once per mount (HEAD /weaknet/v1/capabilities), no polling.
  // Bucket table (design D4, revised in the 2026-09-11 de-noise pass): after the read-only capabilities exemption, 2xx+json = alive;
  // 2xx+html = SPA fallback false-positive (reverse-proxy block missing) / other 4xx (old serve 401/Host gate 403) = also alive / 5xx|reject = down.
  // Hidden in DEV: vite has no /weaknet proxy (judged by port 5173; import.meta.env is not typed in this project).
  const [wnet, setWnet] = useState<'ok' | 'no-proxy' | 'down' | null>(null);
  const showWnet = isAdmin && window.location.port !== '5173';
  useEffect(() => {
    if (!showWnet) return;
    let ignore = false;
    fetch('/weaknet/v1/capabilities', { method: 'HEAD' })
      .then((r) => {
        if (ignore) return;
        const ct = r.headers.get('content-type') || '';
        setWnet(r.ok ? (ct.includes('json') ? 'ok' : 'no-proxy') : r.status >= 500 ? 'down' : 'ok');
      })
      .catch(() => { if (!ignore) setWnet('down'); });
    return () => { ignore = true; };
  }, [showWnet]);
  const handleLogout = () => {
    clearToken();
    navigate('/login');
  };
  return (
    <div className="layout">
      {notices && <div className="notice-banner">ℹ️ {notices}</div>}
      <header className="header">
        <span className="logo"><span className="logo-mark" aria-hidden="true"><Radio size={16} /></span> MediaServo</span>
        <div className="header-right">
          <FloorNotifChips />
          <span className="version">
            {username ? `${username}${role ? ` [${role}]` : ''} · ` : ''}v0.1.0
          </span>
          {token && (
            <button className="logout-btn" onClick={handleLogout}>Logout</button>
          )}
        </div>
      </header>
      <div className="main">
        <nav className="sidebar" aria-label="primary">
          <div className="nav-head">
            <span className="nav-title">Navigation</span>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Overview</span>
            <NavLink to="/" end className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <LayoutDashboard size={15} /> Overview
            </NavLink>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Surveillance</span>
            {/* D11=B: prototype shape kept; unavailable domains are disabled WITH a reason */}
            {canMonitor ? (
              <a href="/monitor/" className="nav-item" title="Monitor: realtime surveillance wall (same-origin app, shared login)">
                <MonitorCog size={15} /> Monitor
              </a>
            ) : (
              <span className="nav-item disabled" title="Monitor: requires admin/dispatcher" aria-disabled="true">
                <MonitorCog size={15} /> Monitor
              </span>
            )}
            <span className="nav-item disabled" title="Recording: deferred until recording-recon unlocks RECORDING_READY" aria-disabled="true">
              <FileAudio size={15} /> Recording
            </span>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Communication</span>
            {canMonitor && (
              <NavLink to="/intercom" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
                <Radio size={15} /> Intercom
                {heldCount > 0 && <span className="nav-badge">{heldCount}</span>}
              </NavLink>
            )}
            <span className="nav-item disabled" title="Meeting: deferred (board B-M1 pending)" aria-disabled="true">
              <Users size={15} /> Meeting
            </span>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Control</span>
            <span className="nav-item disabled" title="Remote: deferred (board B-R1 + B-lease server batch)" aria-disabled="true">
              <Sliders size={15} /> Remote
            </span>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Administration</span>
            {/* D12=A: one Admin entry; spokes live inside the hub */}
            <NavLink to="/admin" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <SettingsIcon size={15} /> Admin
            </NavLink>
            {showWnet && (
              <a href="/weaknet/" target="_blank" rel="noopener noreferrer"
                 className={wnet ? 'nav-item dim' : 'nav-item'}
                 title={wnet === 'no-proxy' ? 'Reverse-proxy block missing (check Caddy)' : wnet === 'down' ? 'weaknet serve not running' : undefined}>
                <Sliders size={15} /> Weaknet Panel
              </a>
            )}
          </div>
          <div className="nav-foot">v0.1.0 · admin</div>
        </nav>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
