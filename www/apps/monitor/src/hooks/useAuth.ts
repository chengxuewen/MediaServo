import { useEffect, useState } from 'react';
import { getRole, getToken, getUsername, subscribeAuth } from '@mediaservo/client';

/**
 * Auth state hook — token/role/username change with setToken/clearToken (same tab).
 * Reuses the shared @mediaservo/client auth surface verbatim (no SDK re-creation);
 * mirrors admin hooks/useAuth.ts but imports the SDK directly.
 */
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
  // admin REST admits admin|dispatcher (server auth_middleware); operator has no
  // monitor wall access — the frontend tightens accordingly (mirrors admin I1).
  const canMonitor = isAdmin || isDispatcher;

  return { token, role, username, isAdmin, isDispatcher, canMonitor };
}