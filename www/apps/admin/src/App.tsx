import { Routes, Route, Navigate } from 'react-router-dom';
import type { ReactNode } from 'react';
import Layout from './components/Layout';
import Login from './pages/Login';
import Dashboard from './pages/Dashboard';
import Settings from './pages/Settings';
import Vehicles from './pages/Vehicles';
import Devices from './pages/Devices';
import Accounts from './pages/Accounts';
import ApiKeys from './pages/ApiKeys';
import Intercom from './pages/Intercom';
import IntercomSession from './pages/IntercomSession';
import Rooms from './pages/Rooms';
import AdminHub from './pages/AdminHub';
import Evidence from './pages/Evidence';
// A (shell-stays): monitor realtime face as an in-shell SPA route (shared source).
import MonitorPage from '@monitor/pages/Home';
import { FloorNotifProvider } from './notifications';
import { hasToken } from './api/client';
import { useAuth } from './hooks/useAuth';

/** Route guard — redirect to /login when no token (login page is public). */
function RequireAuth({ children }: { children: ReactNode }) {
  if (!hasToken()) return <Navigate to="/login" replace />;
  return <>{children}</>;
}

/** Route guard — non-admin hitting device/account management is redirected to / with a notice. */
function RequireAdmin({ children }: { children: ReactNode }) {
  const { isAdmin } = useAuth();
  if (!isAdmin) return <Navigate to="/" replace state={{ notice: 'Admin access required' }} />;
  return <>{children}</>;
}

export default function App() {
  return (
    <FloorNotifProvider>
        <Routes>
      <Route path="/login" element={<Login />} />
      <Route element={<RequireAuth><Layout /></RequireAuth>}>
        <Route path="/" element={<Dashboard />} />
        <Route path="/devices" element={<RequireAdmin><Devices /></RequireAdmin>} />
        <Route path="/accounts" element={<RequireAdmin><Accounts /></RequireAdmin>} />
        <Route path="/apikeys" element={<RequireAdmin><ApiKeys /></RequireAdmin>} />
        <Route path="/audio" element={<Navigate to="/intercom" replace />} />
        <Route path="/monitor" element={<MonitorPage />} />
        <Route path="/intercom" element={<Intercom />} />
        <Route path="/intercom/:roomId" element={<IntercomSession />} />
        <Route path="/rooms" element={<Rooms />} />
        <Route path="/admin" element={<AdminHub />} />
        <Route path="/evidence" element={<Evidence />} />
        <Route path="/vehicles" element={<Vehicles />} />
        <Route path="/settings" element={<Settings />} />
      </Route>
    </Routes>
        </FloorNotifProvider>
  );
}
