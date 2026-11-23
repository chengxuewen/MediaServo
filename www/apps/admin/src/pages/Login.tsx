import { useState, type FormEvent } from 'react';
import { Navigate, useNavigate } from 'react-router-dom';
import { Button, Input } from '@mediaservo/react';
import { login, setToken, hasToken } from '../api/client';

/**
 * W5 (admin-prototype-restyle): login card in the prototype language
 * (Settings "Account Login" card: title + cockpit-account note + inputs +
 * primary action + alert row). The real auth flow is unchanged — the
 * prototype's in-Settings session switch is simulated and stays out.
 */
export default function Login() {
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [loggingIn, setLoggingIn] = useState(false);
  const navigate = useNavigate();

  if (hasToken()) return <Navigate to="/" replace />;

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!username.trim() || !password) return;
    setLoggingIn(true);
    setError(null);
    try {
      const resp = await login(username.trim(), password);
      setToken(resp.token);
      // role-aware default view: operator has no admin REST access (I1) → land on Settings instead of triggering a Dashboard 401
      navigate(resp.role === 'admin' || resp.role === 'dispatcher' ? '/' : '/settings');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed');
    } finally {
      setLoggingIn(false);
    }
  };

  return (
    <div style={{ minHeight: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'var(--ms-color-bg)' }}>
      <form
        onSubmit={handleSubmit}
        style={{
          width: 320,
          background: 'var(--ms-color-surface)',
          border: '1px solid var(--ms-color-border)',
          borderRadius: 'var(--ms-radius-md)',
          padding: 'var(--ms-space-6)',
          boxShadow: 'var(--ms-shadow-1)',
          display: 'flex',
          flexDirection: 'column',
          gap: 'var(--ms-space-3)',
        }}
      >
        <h1 style={{ fontSize: 'var(--ms-font-size-md)', fontWeight: 'var(--ms-font-weight-semibold)', textAlign: 'center', margin: 0 }}>
          Account Login
        </h1>
        <p role="note" style={{ fontSize: 'var(--ms-font-size-xs)', color: 'var(--ms-color-text-tertiary)', textAlign: 'center', margin: 0 }}>
          Sign in with a cockpit account (viewer / operator / dispatcher / admin).
        </p>
        <Input
          type="text"
          placeholder="username"
          aria-label="Username"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          autoComplete="username"
          autoFocus
        />
        <Input
          type="password"
          placeholder="password"
          aria-label="Password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          autoComplete="current-password"
        />
        {error && (
          <p role="alert" style={{ color: 'var(--ms-color-danger)', fontSize: 'var(--ms-font-size-sm)', margin: 0 }}>
            {error}
          </p>
        )}
        <Button variant="primary" type="submit" disabled={loggingIn} disabledReason="Signing in…">
          {loggingIn ? 'Signing in...' : 'Login'}
        </Button>
      </form>
    </div>
  );
}