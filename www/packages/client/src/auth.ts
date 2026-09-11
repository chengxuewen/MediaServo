// ── 认证/JWT 纯面（P0/D270-R1 自 apps/admin api/client.ts 提纯——逻辑零改动）──
// 键名/语义与原实现逐字一致；login 增可选 baseUrl 参数（默认同源，admin 行为不变）。

const LOGIN_URL = '/api/auth/login';
const TOKEN_KEY = 'mediaservo_admin_token';

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

/** JWT exp（秒）是否已过期；无 exp/解析失败视为失效。 */
export function isTokenExpired(token: string): boolean {
  const claims = parseToken(token);
  return !claims?.exp || claims.exp * 1000 <= Date.now();
}

// ── JWT claims + auth 状态（H3: dispatcher 角色感知渲染）──────────────────────

export interface JwtClaims {
  sub?: string;
  role?: string;
  vehicles?: string[];
  iat?: number;
  exp?: number;
}

/** base64url 解码 JWT payload（无库依赖; 结构异常 → null）。 */
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

/** 同标签页内 token 变更通知（login/logout 后 Layout/nav 重渲染）。 */
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
