// ── admin REST 面（/api/admin 前缀消费方）──
// P0/D270-R1: auth/JWT 纯面与 wire 类型已提纯至 @mediaservo/client（单一真源），
// 本文件 re-export 保持全部既有消费方 import 路径零改动；request()/REST 函数为 admin 专属留此。
import { getToken, isTokenExpired, clearToken } from '@mediaservo/client';
import type {
  DeviceListResponse,
  StatsResponse,
  SfuRoomsResponse,
  SfuStats,
  VehicleStatusResponse,
  AdminDeviceListResponse,
  AdminDeviceSecret,
  AdminDeviceRevoked,
  PendingDeviceListResponse,
  PendingDeviceApproved,
  AdminAccountListResponse,
  AdminAccountCreated,
  AdminAccountUpdated,
  AdminAccountDeleted,
  PskResponse,
} from '@mediaservo/client';

export * from '@mediaservo/client';

const BASE = '/api/admin';

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

// ── 待批准队列（device-enroll 手动档）──────────────────────────────────
export async function getPendingDevices(): Promise<PendingDeviceListResponse> {
  return request('/devices/pending');
}

export async function approvePendingDevice(deviceId: string, name?: string): Promise<PendingDeviceApproved> {
  const body: { device_id: string; name?: string } = { device_id: deviceId };
  if (name && name.trim()) body.name = name.trim();
  return request('/devices/approve', { method: 'POST', body: JSON.stringify(body) });
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

export async function getPsk(): Promise<PskResponse> {
  return request('/psk');
}

export async function rotatePsk(password?: string): Promise<PskResponse> {
  const body = password ? { password } : {};
  return request('/psk', { method: 'POST', body: JSON.stringify(body) });
}

export function connectEvents(): WebSocket {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = getToken();
  const url = `${protocol}//${window.location.host}/api/admin/events`;
  // ponytail: pass token via query param for WS (no custom headers in browser WebSocket)
  return new WebSocket(token ? `${url}?token=${encodeURIComponent(token)}` : url);
}
