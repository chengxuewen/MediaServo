import { Routes, Route, Navigate } from 'react-router-dom';
import type { ReactNode } from 'react';
import Layout from './components/Layout';
import Login from './pages/Login';
import Dashboard from './pages/Dashboard';
import Settings from './pages/Settings';
import Audio from './pages/Audio';
import Vehicles from './pages/Vehicles';
import Devices from './pages/Devices';
import Accounts from './pages/Accounts';
import { hasToken } from './api/client';
import { useAuth } from './hooks/useAuth';

/** 路由守卫 — 无 token 重定向 /login（登录页公开）。 */
function RequireAuth({ children }: { children: ReactNode }) {
  if (!hasToken()) return <Navigate to="/login" replace />;
  return <>{children}</>;
}

/** 路由守卫 — 非 admin 访问设备/账号管理重定向 / 并提示。 */
function RequireAdmin({ children }: { children: ReactNode }) {
  const { isAdmin } = useAuth();
  if (!isAdmin) return <Navigate to="/" replace state={{ notice: 'Admin access required' }} />;
  return <>{children}</>;
}

export default function App() {
  return (
    <Routes>
      <Route path="/login" element={<Login />} />
      <Route element={<RequireAuth><Layout /></RequireAuth>}>
        <Route path="/" element={<Dashboard />} />
        <Route path="/devices" element={<RequireAdmin><Devices /></RequireAdmin>} />
        <Route path="/accounts" element={<RequireAdmin><Accounts /></RequireAdmin>} />
        <Route path="/audio" element={<Audio />} />
        <Route path="/vehicles" element={<Vehicles />} />
        <Route path="/settings" element={<Settings />} />
      </Route>
    </Routes>
  );
}
