import { Outlet, useNavigate } from 'react-router-dom';
import { MonitorCog } from 'lucide-react';
import { useAuth } from '../hooks/useAuth';
import { clearToken } from '@mediaservo/client';
import './Layout.css';

/**
 * Thin-shell topbar (P3 slice ①): brand + identity + logout only. No sidebar —
 * the monitor realtime surface (stream tree + wall + timeline) lands in slice ②③.
 * The B-shell package replaces this shell once approved (PLAN §8 壳迁移批).
 * Header structure mirrors admin (.header .version / .logout-btn) so the shared
 * login helper's identity assertion works unchanged.
 */
export default function Layout() {
  const { role, username, token } = useAuth();
  const navigate = useNavigate();
  const handleLogout = () => {
    clearToken();
    navigate('/login');
  };
  return (
    <div className="monitor-shell">
      <header className="header">
        <span className="logo">
          <span className="logo-mark" aria-hidden="true"><MonitorCog size={16} /></span>
          MediaServo Monitor
        </span>
        <div className="header-right">
          <span className="version">
            {username ? `${username}${role ? ` [${role}]` : ''} · ` : ''}v0.1.0
          </span>
          {token && (
            <button className="logout-btn" onClick={handleLogout}>Logout</button>
          )}
        </div>
      </header>
      <main className="monitor-content">
        <Outlet />
      </main>
    </div>
  );
}