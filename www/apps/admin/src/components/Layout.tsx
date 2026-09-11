import { useEffect, useState } from 'react';
import { NavLink, Outlet, useNavigate, useLocation } from 'react-router-dom';
import { useAuth } from '../hooks/useAuth';
import { clearToken } from '../api/client';
import './Layout.css';
import { Radio, LayoutDashboard, Mic, Car, MonitorCog, Users, Settings as SettingsIcon, Sliders } from 'lucide-react';

export default function Layout() {
  const { role, username, canMonitor, token, isAdmin } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const notices = (location.state as { notice?: string } | null)?.notice;
  // T8 弱网面板入口卡：每 mount 探活一次（HEAD /weaknet/v1/capabilities），无轮询。
  // 桶表(design D4，2026-09-11 消噪刀修订)：capabilities 无凭证只读豁免后 2xx+json=活；
  // 2xx+html=SPA fallback 假亮(反代段缺位) / 其余 4xx(老版 serve 仍 401/Host 门 403)=也证明活着 / 5xx|reject=down。
  // DEV 隐藏：vite 无 /weaknet 代理（按 5173 端口判据，工程未类型化 import.meta.env）。
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
          {/* H3: 音频会议 + 多车监控 = G3 can_status 角色（operator/admin/dispatcher） */}
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
          {/* Devices/Accounts 管理 = 仅 admin（与 RequireAdmin 守卫一致） */}
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
          {/* 弱网面板 = 纯外链（App.tsx 路由表不加 path），新标签打开 serve 面板 */}
          {showWnet && (
            <a href="/weaknet/" target="_blank" rel="noopener noreferrer"
               className={wnet ? 'nav-item dim' : 'nav-item'}
               title={wnet === 'no-proxy' ? '反代段缺位（检查 Caddy）' : wnet === 'down' ? 'weaknet serve 未运行' : undefined}>
              <Sliders size={15} /> 弱网面板
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
