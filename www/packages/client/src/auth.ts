// ── Auth/JWT pure surface (P0/D270-R1: extracted from apps/admin api/client.ts — logic unchanged) ──
// Keys/semantics are verbatim from the original; login gains an optional baseUrl parameter (defaults to same-origin; admin behavior unchanged).

const LOGIN_URL = '/api/auth/login';
const TOKEN_KEY = 'mediaservo_admin_token';

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

/** Whether the JWT exp (seconds) has passed; missing exp or a parse failure counts as expired. */
export function isTokenExpired(token: string): boolean {
  const claims = parseToken(token);
  return !claims?.exp || claims.exp * 1000 <= Date.now();
}

// ── JWT claims + auth state (H3: dispatcher role-aware rendering) ─────────────

export interface JwtClaims {
  sub?: string;
  role?: string;
  vehicles?: string[];
  iat?: number;
  exp?: number;
}

/** base64url-decode the JWT payload (no library dependency; malformed structure → null). */
export function parseToken(token: string): JwtClaims | null {
  try {
    const payload = token.split('.')[1];
    if (!payload) return null;
    return JSON.parse(atob(payload.replace(/-/g, '+').replace(/_/g, '/')));
  } catch {
    return null;
  }
}

export function getRole(): string | null {
  const token = getToken();
  return token ? (parseToken(token)?.role ?? null) : null;
}

export function getUsername(): string | null {
  const token = getToken();
  return token ? (parseToken(token)?.sub ?? null) : null;
}

/** Same-tab token change notification (Layout/nav re-render after login/logout). */
const authListeners = new Set<() => void>();
function notifyAuth() { authListeners.forEach((fn) => fn()); }
export function subscribeAuth(fn: () => void): () => void {
  authListeners.add(fn);
  return () => { authListeners.delete(fn); };
}

export interface LoginResponse { token: string; username: string; role: string; expires_in_secs: number; }

export async function login(username: string, password: string, baseUrl = ''): Promise<LoginResponse> {
  const res = await fetch(`${baseUrl}${LOGIN_URL}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) throw new Error(res.status === 401 ? 'Invalid username or password' : `Login failed: ${res.status}`);
  return res.json();
}

export function setToken(token: string) { localStorage.setItem(TOKEN_KEY, token); notifyAuth(); }
export function clearToken() { localStorage.removeItem(TOKEN_KEY); notifyAuth(); }
export function hasToken(): boolean { return !!getToken(); }
