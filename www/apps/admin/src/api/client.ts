// ── admin REST surface (consumer of the /api/admin prefix) ──
// P0/D270-R1: the auth/JWT pure surface and wire types were extracted to @mediaservo/client (single source),
// this file re-exports them so existing consumer import paths stay unchanged; request()/REST functions stay admin-only here.
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
  FloorSnapshot,
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
    // 401 self-heal: only clear the login state for invalid/expired tokens (valid-but-denied e.g. operator keeps the error state — roles-operator.spec)
    const token = getToken();
    if (!token || isTokenExpired(token)) {
      clearToken();
      const base = window.location.pathname.startsWith('/admin') ? '/admin' : '';
      window.location.href = `${base}/login`;
    }
    throw new Error('Authentication required — please sign in');
  }
      // non-2xx: backend error body is {error} — throw the message so callers can see the error branch (C15)
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

// SB-3/T5 (talk-app): managed rooms drive the intercom board's room list
// (kind=talk rooms are the designated floor rooms).
export interface FloorOverrideEntry {
  max_peers?: number;
  max_queue?: number;
  hold_timeout_secs?: number;
}

export interface ManagedRoomEntry {
  id: string;
  kind: string;
  video?: boolean | null;
  /** SB-2 per-room floor limits overlay (absent = global resolution). */
  floor?: FloorOverrideEntry | null;
}

export async function listManagedRooms(): Promise<{ rooms: ManagedRoomEntry[]; count: number }> {
  return request('/managed-rooms');
}

export async function createManagedRoom(body: {
  id: string;
  kind: string;
  video?: boolean | null;
  floor?: FloorOverrideEntry | null;
}): Promise<{ room: ManagedRoomEntry }> {
  return request('/managed-rooms', { method: 'POST', body: JSON.stringify(body) });
}

export async function updateManagedRoom(
  id: string,
  body: { kind?: string; video?: boolean | null; floor?: FloorOverrideEntry | null },
): Promise<{ room: ManagedRoomEntry }> {
  return request(`/managed-rooms/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify(body),
  });
}

export async function deleteManagedRoom(id: string): Promise<void> {
  await request(`/managed-rooms/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// T9 evidence retention (K5/K14/7.3): declare (consent + audited), list,
// download (audited), withdraw (admin-only, irreversible). Raw audio body.
export interface EvidenceItemEntry {
  id: string;
  room_id: string;
  actor: string;
  label?: string;
  ts: number;
  retain_until: number;
  size: number;
}

export async function retainEvidence(
  blob: Blob,
  roomId: string,
  label = '',
): Promise<{ item: EvidenceItemEntry }> {
  const token = getToken();
  const res = await fetch(
    `${BASE}/evidence?room_id=${encodeURIComponent(roomId)}&label=${encodeURIComponent(label)}`,
    {
      method: 'POST',
      headers: token
        ? { Authorization: `Bearer ${token}`, 'Content-Type': 'audio/webm' }
        : { 'Content-Type': 'audio/webm' },
      body: blob,
    },
  );
  if (res.status === 401) {
    clearToken();
    throw new Error('Authentication required');
  }
  if (!res.ok) {
    let msg = `HTTP ${res.status}`;
    try {
      const j = (await res.json()) as { error?: string };
      if (j?.error) msg = j.error;
    } catch {
      /* keep status text */
    }
    throw new Error(msg);
  }
  return res.json();
}

export async function listEvidence(): Promise<{
  items: EvidenceItemEntry[];
  count: number;
  retention_secs: number;
  source: string;
}> {
  return request('/evidence');
}

export async function fetchEvidenceBlob(id: string): Promise<Blob> {
  const token = getToken();
  const res = await fetch(`${BASE}/evidence/${encodeURIComponent(id)}`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.blob();
}

export async function deleteEvidence(id: string): Promise<void> {
  await request(`/evidence/${encodeURIComponent(id)}`, { method: 'DELETE' });
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
  const qs = params.toString();
  return request(`/sfu/stats${qs ? `?${qs}` : ''}`);
}

// P4-2 (ADJ-1=B): floor snapshot is READ-ONLY here — the control face lives in
// the voice dispatch console (/dispatch/:room); a 404 means the room is not a
// designated talk room (floor disabled) and the caller should stop polling it.
export async function getFloorSnapshot(roomId: string): Promise<FloorSnapshot> {
  return request(`/floor/${encodeURIComponent(roomId)}`);
}

export async function getVehicleStatus(): Promise<VehicleStatusResponse> {
  return request('/status');
}
// ── Device management (AdminState) ──────────────────────────────────────
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

// ── Pending approval queue (device-enroll manual mode) ─────────────────
export async function getPendingDevices(): Promise<PendingDeviceListResponse> {
  return request('/devices/pending');
}

export async function approvePendingDevice(deviceId: string, name?: string): Promise<PendingDeviceApproved> {
  const body: { device_id: string; name?: string } = { device_id: deviceId };
  if (name && name.trim()) body.name = name.trim();
  return request('/devices/approve', { method: 'POST', body: JSON.stringify(body) });
}

// ── Account management (AdminState) ─────────────────────────────────────
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

// ── API Keys management (accountless-client-auth T1; wire types stay admin-only here) ──
export interface AdminApiKey {
  key_id: string;
  role: string;
  vehicles: string[];
  label: string | null;
}

export interface AdminApiKeyListResponse { api_keys: AdminApiKey[] }
export interface AdminApiKeyCreated { key_id: string; secret: string; note: string }
export interface AdminApiKeyRevoked { revoked: string }

export async function getApiKeys(): Promise<AdminApiKeyListResponse> {
  return request('/apikeys');
}

export async function createApiKey(keyId: string, role: string, vehicles: string[], label: string | null): Promise<AdminApiKeyCreated> {
  return request('/apikeys', { method: 'POST', body: JSON.stringify({ key_id: keyId, role, vehicles, label }) });
}

export async function revokeApiKey(keyId: string): Promise<AdminApiKeyRevoked> {
  return request(`/apikeys/${encodeURIComponent(keyId)}`, { method: 'DELETE' });
}

// ── PSK management (psk-admin-management — admin-only endpoints) ──────────

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
