const BASE = '/api/admin';
const LOGIN_URL = '/api/auth/login';
const TOKEN_KEY = 'mediaservo_admin_token';

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

function headers(): Record<string, string> {
  const token = getToken();
  return token ? { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' } : { 'Content-Type': 'application/json' };
}

async function request<T>(path: string, opts?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { ...opts, headers: { ...headers(), ...opts?.headers } });
  if (res.status === 401) {
    // 401 自愈: 仅对失效/过期 token 清登录态跳转（valid-but-denied 如 operator 保留错误态 — roles-operator.spec）
    const token = getToken();
    if (!token || isTokenExpired(token)) {
      clearToken();
      const base = window.location.pathname.startsWith('/admin') ? '/admin' : '';
      window.location.href = `${base}/login`;
    }
    throw new Error('Authentication required — please sign in');
  }
      // 非 2xx: 后端错误体为 {error} — 直接抛消息，让调用方可见错误分支（C15）
      if (!res.ok) {
        const body = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(body?.error ?? `Request failed (${res.status})`);
      }
  return res.json();
}

/** JWT exp（秒）是否已过期；无 exp/解析失败视为失效。 */
function isTokenExpired(token: string): boolean {
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

// Types
export interface Consumer { peer_id: string; connected_since: string; }
export interface StreamSnapshot { stream_id: string; consumers: Consumer[]; online: boolean; }
export interface DeviceSnapshot { device_id: string; online_since: string; streams: StreamSnapshot[]; }
export interface DeviceListResponse { devices: DeviceSnapshot[]; total_devices: number; }
export interface StatsResponse { active_rooms: number; total_peers: number; active_connections: number; }

// H3: SFU 房间摘要（音频会议面板数据源）。
export interface SfuRoom {
  room_id: string;
  participants: number;
  producers: number;
  consumers: number;
  audio: boolean;
  producer_ids: string[];
  consumer_ids: string[];
}
export interface SfuRoomsResponse { rooms: SfuRoom[]; }

// H3: SfuStats（镜像 WS 信令 SfuStats — H2 协议的管理面）。
export interface SfuStats {
  producer_id?: string;
  consumer_id?: string;
  kind?: 'audio' | 'video';
  byte_count: number;
  packet_count: number;
  score: number;
}

// H3: 多车状态上报（StatusReport wire 镜像 — E3）。
export interface TopicFlow { topic: string; fps: number; bps: number; last_ts_mono_ns: number; frames: number; stalled: boolean; }
export interface StreamFlow { id: string; bytes_sent: number; frames_encoded: number; frame_width: number; frame_height: number; connected: boolean; }
export interface ProcessState { name: string; running: boolean; expected: boolean; }
export interface ChildSignal { src: string; connected: boolean; last_msg_secs: number; }
export interface SignalStatus {
  remote_connected: boolean;
  remote_since_secs?: number;
  remote_peer_id: string;
  children: ChildSignal[];
  agent_uptime_secs: number;
}
export interface StatusReport {
  room_id: string;
  topics: TopicFlow[];
  streams: StreamFlow[];
  processes: ProcessState[];
  signal: SignalStatus;
  ts: number;
  config_version: number;
}
export interface VehicleStatusResponse { vehicles: { room_id: string; report: StatusReport }[]; }
// Device/account admin（AdminState — 授权设备与账号管理端点）
export interface AdminDevice { device_id: string; }
export interface AdminDeviceListResponse { devices: AdminDevice[]; count: number; }
export interface AdminDeviceSecret { device_id: string; secret: string; secret_hash: string; note: string; }
export interface AdminDeviceRevoked { device_id: string; revoked: boolean; }
export type AccountRole = 'viewer' | 'operator' | 'admin' | 'dispatcher';
export interface AdminAccount { username: string; role: string; vehicles: string[]; }
export interface AdminAccountListResponse { accounts: AdminAccount[]; count: number; }
export interface AdminAccountCreated { created: string; }
export interface AdminAccountUpdated { updated: string; }
export interface AdminAccountDeleted { deleted: string; }


// API functions
export async function getDevices(): Promise<DeviceListResponse> {
  return request('/rooms');
}

export async function getStats(): Promise<StatsResponse> {
  return request('/stats');
}

export async function deleteRoom(roomId: string): Promise<void> {
  await request(`/rooms/${roomId}`, { method: 'DELETE' });
}

export async function getSfuRooms(): Promise<SfuRoomsResponse> {
  return request('/sfu/rooms');
}

export async function getSfuStats(producerId?: string, consumerId?: string): Promise<SfuStats> {
  const params = new URLSearchParams();
  if (producerId) params.set('producer_id', producerId);
  if (consumerId) params.set('consumer_id', consumerId);
  return request(`/sfu/stats?${params.toString()}`);
}

export async function getVehicleStatus(): Promise<VehicleStatusResponse> {
  return request('/status');
}
// ── 设备管理（AdminState）──────────────────────────────────────────────
export async function getAdminDevices(): Promise<AdminDeviceListResponse> {
  return request('/devices');
}

export async function registerDevice(deviceId: string, secret?: string): Promise<AdminDeviceSecret> {
  const body: { device_id: string; secret?: string } = { device_id: deviceId };
  if (secret && secret.trim()) body.secret = secret.trim();
  return request('/devices', { method: 'POST', body: JSON.stringify(body) });
}

export async function revokeDevice(deviceId: string): Promise<AdminDeviceRevoked> {
  return request(`/devices/${encodeURIComponent(deviceId)}`, { method: 'DELETE' });
}

export async function resetDeviceSecret(deviceId: string): Promise<AdminDeviceSecret> {
  return request(`/devices/${encodeURIComponent(deviceId)}/reset-secret`, { method: 'POST' });
}

// ── 账号管理（AdminState）──────────────────────────────────────────────
export async function getAdminAccounts(): Promise<AdminAccountListResponse> {
  return request('/accounts');
}

export async function createAccount(username: string, password: string, role: string, vehicles: string[]): Promise<AdminAccountCreated> {
  return request('/accounts', { method: 'POST', body: JSON.stringify({ username, password, role, vehicles }) });
}

export async function updateAccount(username: string, patch: { role?: string; vehicles?: string[]; new_password?: string }): Promise<AdminAccountUpdated> {
  return request(`/accounts/${encodeURIComponent(username)}`, { method: 'PUT', body: JSON.stringify(patch) });
}

export async function deleteAccount(username: string): Promise<AdminAccountDeleted> {
  return request(`/accounts/${encodeURIComponent(username)}`, { method: 'DELETE' });
}

// ── PSK 管理（psk-admin-management — admin-only 端点）──────────────────────

export interface PskResponse {
  psk: string;
  hint: string;
}

export async function getPsk(): Promise<PskResponse> {
  return request('/psk');
}

export async function rotatePsk(password?: string): Promise<PskResponse> {
  const body = password ? { password } : {};
  return request('/psk', { method: 'POST', body: JSON.stringify(body) });
}


export interface LoginResponse { token: string; username: string; role: string; expires_in_secs: number; }

export async function login(username: string, password: string): Promise<LoginResponse> {
  const res = await fetch(LOGIN_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) throw new Error(res.status === 401 ? 'Invalid username or password' : `Login failed: ${res.status}`);
  return res.json();
}

export function connectEvents(): WebSocket {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = getToken();
  const url = `${protocol}//${window.location.host}/api/admin/events`;
  // ponytail: pass token via query param for WS (no custom headers in browser WebSocket)
  return new WebSocket(token ? `${url}?token=${encodeURIComponent(token)}` : url);
}

export function setToken(token: string) { localStorage.setItem(TOKEN_KEY, token); notifyAuth(); }
export function clearToken() { localStorage.removeItem(TOKEN_KEY); notifyAuth(); }
export function hasToken(): boolean { return !!getToken(); }
