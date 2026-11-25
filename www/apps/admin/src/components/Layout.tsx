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
  // V22 (prototype topbar): brand is followed by the CURRENT page name.
  const pageLabel = (() => {
    const p = location.pathname.replace(/\/+$/, '').split('/').pop() || '';
    const map: Record<string, string> = {
      '': 'Overview', monitor: 'Monitor Center', intercom: 'Intercom', admin: 'Admin',
      settings: 'Settings', devices: 'Devices', rooms: 'Rooms', accounts: 'Accounts',
      'api-keys': 'API keys', evidence: 'Evidence', vehicles: 'Vehicles',
    };
    return map[p] ?? (p ? p.replace(/-/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()) : 'Overview');
  })();
  // Mainstream practice: keep the browser tab title in sync with the current page.
  useEffect(() => { document.title = `${pageLabel} · MediaServo`; }, [pageLabel]);
  // V21 (prototype nav): collapsible sidebar -> 58px icon rail; auto-rail at <=900px.
  const [navCollapsed, setNavCollapsed] = useState(() => { try { return localStorage.getItem('msrv_nav_collapsed') === '1'; } catch { return false; } });
  const [narrow, setNarrow] = useState(() => window.matchMedia('(max-width: 1024px)').matches);
  useEffect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const on = () => setNarrow(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  const navMin = navCollapsed || narrow;
  useEffect(() => { try { localStorage.setItem('msrv_nav_collapsed', navCollapsed ? '1' : '0'); } catch { /* silent */ } }, [navCollapsed]);
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
  // V32 portal topbar: status chip shows the role short; avatar shows initials.
  const [userMenu, setUserMenu] = useState(false);
  useEffect(() => {
    if (!userMenu) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setUserMenu(false); };
    const onClick = (e: MouseEvent) => { if (!(e.target as HTMLElement).closest('.user-wrap')) setUserMenu(false); };
    window.addEventListener('keydown', onKey);
    window.addEventListener('mousedown', onClick);
    return () => { window.removeEventListener('keydown', onKey); window.removeEventListener('mousedown', onClick); };
  }, [userMenu]);
  const roleShort = role === 'admin' ? 'AD' : role === 'dispatcher' ? 'DP' : role === 'operator' ? 'OP' : 'NA';
  const initials = (username ?? 'user').slice(0, 2).toUpperCase();
  return (
    <div className="layout">
      {notices && <div className="notice-banner">ℹ️ {notices}</div>}
      <header className="header">
        <div className="header-left">
          <span className="logo"><span className="logo-mark" aria-hidden="true"><Radio size={16} /></span><span className="logo-text">MediaServo</span></span>
          <span className="header-rule" aria-hidden="true" />
          <span className="header-page" aria-current="page">{pageLabel}</span>
        </div>
        <div className="header-right">
          <FloorNotifChips />
          {/* V32 (portal topbar): bell + status chip + user chip menu replace the
              inline "user [role] · v · Logout" text. The role/version text stays as an
              accessible-only node (.version) so role assertions keep working. */}
          <span className="version visually-hidden">{username ? `${username}${role ? ` [${role}]` : ''} · ` : ''}v0.1.0</span>
          <button className="header-bell" aria-label="Notifications" title="Notifications">
            <span aria-hidden="true">🔔</span>
          </button>
          <button className="header-status" title="System status: connection + role">
            <span className={`st-dot${token ? ' on' : ''}`} aria-hidden="true" />
            <b>{roleShort}</b>
          </button>
          {token && (
            <div className="user-wrap">
              <button className="user-chip" aria-haspopup="menu" aria-expanded={userMenu}
                aria-label="User menu" onClick={() => setUserMenu((o) => !o)}>
                <span className="avatar" aria-hidden="true">{initials}</span>
                <span className="user-name">{username ?? 'user'}</span>
                <span className="caret" aria-hidden="true">▾</span>
              </button>
              {userMenu && (
                <div className="user-menu" role="menu">
                  <div className="um-head">{username}{role ? ` · ${role}` : ''} · v0.1.0</div>
                  <button role="menuitem" className="um-item logout-btn" onClick={handleLogout}>Sign out</button>
                </div>
              )}
            </div>
          )}
        </div>
      </header>
      <div className={`main${navMin ? ' nav-min' : ''}`}>
        <nav className="sidebar" aria-label="primary">
          <div className="nav-head">
            <span className="nav-title">Navigation</span>
            <button className="nav-collapse" onClick={() => setNavCollapsed((c) => !c)}
              title={navMin ? 'Expand navigation' : 'Collapse navigation'}
              aria-label={navMin ? 'Expand navigation' : 'Collapse navigation'} aria-expanded={!navMin}>
              {navMin ? '›' : '‹'}
            </button>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Overview</span>
            <NavLink to="/" end title="Overview" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <LayoutDashboard size={15} /><span className="nav-label">Overview</span>
            </NavLink>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Surveillance</span>
            {/* D11=B: prototype shape kept; unavailable domains are disabled WITH a reason */}
            {canMonitor ? (
              <NavLink to="/monitor" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'} title="Monitor: realtime surveillance wall (in-shell)">
                <MonitorCog size={15} /><span className="nav-label">Monitor</span>
              </NavLink>
            ) : (
              <span className="nav-item disabled" title="Monitor: requires admin/dispatcher" aria-disabled="true">
                <MonitorCog size={15} /><span className="nav-label">Monitor</span>
              </span>
            )}
            <span className="nav-item disabled" title="Recording: deferred until recording-recon unlocks RECORDING_READY" aria-disabled="true">
              <FileAudio size={15} /><span className="nav-label">Recording</span>
            </span>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Communication</span>
            {canMonitor && (
              <NavLink to="/intercom" title="Intercom" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
                <Radio size={15} /><span className="nav-label">Intercom</span>
                {heldCount > 0 && <span className="nav-badge">{heldCount}</span>}
              </NavLink>
            )}
            <span className="nav-item disabled" title="Meeting: deferred (board B-M1 pending)" aria-disabled="true">
              <Users size={15} /><span className="nav-label">Meeting</span>
            </span>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Control</span>
            <span className="nav-item disabled" title="Remote: deferred (board B-R1 + B-lease server batch)" aria-disabled="true">
              <Sliders size={15} /><span className="nav-label">Remote</span>
            </span>
          </div>
          <div className="nav-group">
            <span className="nav-group-label">Administration</span>
            {/* D12=A: one Admin entry; spokes live inside the hub */}
            <NavLink to="/admin" title="Admin" className={({ isActive }) => isActive ? 'nav-item active' : 'nav-item'}>
              <SettingsIcon size={15} /><span className="nav-label">Admin</span>
            </NavLink>
            {showWnet && (
              <a href="/weaknet/" target="_blank" rel="noopener noreferrer"
                 className={wnet ? 'nav-item dim' : 'nav-item'}
                 title={wnet === 'no-proxy' ? 'Reverse-proxy block missing (check Caddy)' : wnet === 'down' ? 'weaknet serve not running' : 'Weaknet Panel'}>
                <Sliders size={15} /><span className="nav-label">Weaknet Panel</span>
              </a>
            )}
          </div>
          <div className="nav-foot"><span>v0.1.0 · admin</span></div>
        </nav>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
