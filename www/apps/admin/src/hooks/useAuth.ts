import { useEffect, useState } from 'react';
import { getRole, getToken, getUsername, subscribeAuth } from '../api/client';

/** H3: auth state hook — token/role/username change with setToken/clearToken (same tab). */
export function useAuth() {
  const [token, setTokenState] = useState<string | null>(getToken());
  const [role, setRole] = useState<string | null>(getRole());
  const [username, setUsername] = useState<string | null>(getUsername());

  useEffect(() => {
    return subscribeAuth(() => {
      setTokenState(getToken());
      setRole(getRole());
      setUsername(getUsername());
    });
  }, []);

  const isAdmin = role === 'admin';
  const isDispatcher = role === 'dispatcher';
  // I1 review fix: align with server auth_middleware admission (admin|dispatcher) —
  // G3 can_status includes operator, but admin REST only admits admin/dispatcher;
  // showing the nav to operator would 401 across the board → tighten the frontend; operator falls back to the default view.
  const canMonitor = isAdmin || isDispatcher;

  return { token, role, username, isAdmin, isDispatcher, canMonitor };
}
