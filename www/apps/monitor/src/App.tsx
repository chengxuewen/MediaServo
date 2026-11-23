import { Routes, Route, Navigate } from 'react-router-dom';
import { useEffect, type ReactNode } from 'react';
import Layout from './components/Layout';
import Login from './pages/Login';
import Home from './pages/Home';
import { useAuth } from './hooks/useAuth';
import { isTokenExpired, clearToken } from '@mediaservo/client';

/**
 * Route guard — redirect to /login when no token (login page is public).
 * Thin-shell posture (P3 slice ①): the monitor home makes no REST calls yet
 * (the wall lands in slice ②③), so the guard proactively rejects expired or
 * malformed tokens here rather than waiting for a 401. Slice ③ adds the
 * request()-layer 401 self-heal (mirrors admin api/client.ts) on top of this.
 */
function RequireAuth({ children }: { children: ReactNode }) {
  const { token } = useAuth();
  const expired = token ? isTokenExpired(token) : false;
  useEffect(() => {
    if (token && expired) clearToken();
  }, [token, expired]);
  if (!token || expired) return <Navigate to="/login" replace />;
  return <>{children}</>;
}

export default function App() {
  return (
    <Routes>
      <Route path="/login" element={<Login />} />
      <Route element={<RequireAuth><Layout /></RequireAuth>}>
        <Route path="/" element={<Home />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Route>
    </Routes>
  );
}