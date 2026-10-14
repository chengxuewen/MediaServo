import { useEffect, useState } from 'react';
import { NavLink, Outlet, useNavigate, useLocation } from 'react-router-dom';
import { useAuth } from '../hooks/useAuth';
import { clearToken } from '../api/client';
import './Layout.css';
import { Radio, LayoutDashboard, Mic, Car, MonitorCog, Users, Settings as SettingsIcon, Sliders, KeyRound } from 'lucide-react';

export default function Layout() {
  const { role, username, canMonitor, token, isAdmin } = useAuth();
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
        <span className="logo"><Radio size={18} /> MediaServo Admin</span>
        <div className="header-right">
          <span className="version">
            {username ? `${username}${role ? ` [${role}]` : ''} · ` : ''}v0.1.0
          </span>
          {token && (
            <button className="logout-btn" onClick={handleLogout}>Logout</button>
          )}
        </div>
      </header>
      <div className="main">
        <nav className="sidebar">
          <NavLink to="/" end className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
            <LayoutDashboard size={15} /> Dashboard
          </NavLink>
          {/* H3: audio conference + multi-vehicle monitoring = G3 can_status roles (operator/admin/dispatcher) */}
          {canMonitor && (
            <NavLink to="/audio" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <Mic size={15} /> Audio Conference
            </NavLink>
          )}
          {canMonitor && (
            <NavLink to="/vehicles" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <Car size={15} /> Vehicles
            </NavLink>
          )}
          {/* Devices/Accounts/ApiKeys management = admin only (matches the RequireAdmin guard) */}
          {isAdmin && (
            <NavLink to="/devices" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <MonitorCog size={15} /> Device Management
            </NavLink>
          )}
          {isAdmin && (
            <NavLink to="/accounts" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <Users size={15} /> Account Management
            </NavLink>
          )}
          {isAdmin && (
            <NavLink to="/apikeys" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <KeyRound size={15} /> API Keys
            </NavLink>
          )}
          {/* weaknet panel = pure external link (no path added to the App.tsx route table); opens the serve panel in a new tab */}
          {showWnet && (
            <a href="/weaknet/" target="_blank" rel="noopener noreferrer"
               className={wnet ? 'nav-item dim' : 'nav-item'}
               title={wnet === 'no-proxy' ? 'Reverse-proxy block missing (check Caddy)' : wnet === 'down' ? 'weaknet serve not running' : undefined}>
              <Sliders size={15} /> Weaknet Panel
            </a>
          )}
          <NavLink to="/settings" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
            <SettingsIcon size={15} /> Settings
          </NavLink>
        </nav>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
