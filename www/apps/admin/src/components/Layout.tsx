import { NavLink, Outlet, useNavigate, useLocation } from 'react-router-dom';
import { useAuth } from '../hooks/useAuth';
import { clearToken } from '../api/client';
import './Layout.css';
import { Radio, LayoutDashboard, Mic, Car, MonitorCog, Users, Settings as SettingsIcon } from 'lucide-react';

export default function Layout() {
  const { role, username, canMonitor, token, isAdmin } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const notices = (location.state as { notice?: string } | null)?.notice;
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
