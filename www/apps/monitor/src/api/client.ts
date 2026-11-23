// ── monitor REST surface (consumer of /api/admin; P3 slice ③a) ──
// Auth/JWT + wire types come from @mediaservo/client (single source, re-exported
// so consumer import paths stay stable); request()/REST helpers are monitor-local
// glue mirroring admin api/client.ts. No SDK re-creation.
import { getToken, isTokenExpired, clearToken } from '@mediaservo/client';
import type { DeviceListResponse, SfuRoomsResponse, SfuStats, SfuStatsListResponse } from '@mediaservo/client';

export * from '@mediaservo/client';

const BASE = '/api/admin';

function headers(): Record<string, string> {
  const token = getToken();
  return token ? { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' } : { 'Content-Type': 'application/json' };
}

async function request<T>(path: string, opts?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { ...opts, headers: { ...headers(), ...opts?.headers } });
  if (res.status === 401) {
    // 401 self-heal: only clear login state for invalid/expired tokens (valid-but-denied keeps the error state).
    const token = getToken();
    if (!token || isTokenExpired(token)) {
      clearToken();
      const base = window.location.pathname.startsWith('/monitor') ? '/monitor' : '';
      window.location.href = `${base}/login`;
    }
    throw new Error('Authentication required — please sign in');
  }
  if (!res.ok) {
    const body = (await res.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `Request failed (${res.status})`);
  }
  return res.json();
}

// Device registry / active rooms (device tree status cross-reference).
export async function getDevices(): Promise<DeviceListResponse> {
  return request('/rooms');
}

export async function getSfuRooms(): Promise<SfuRoomsResponse> {
  return request('/sfu/rooms');
}

// B5 corrected contract: no params = full live producer/consumer list ("in the table = alive").
export async function getSfuStatsList(): Promise<SfuStatsListResponse> {
  return request('/sfu/stats');
}

// Single-stream metrics: ?producer_id=<uuid> (underscore; mutually exclusive with consumer_id).
export async function getSfuStats(producerId?: string, consumerId?: string): Promise<SfuStats> {
  const params = new URLSearchParams();
  if (producerId) params.set('producer_id', producerId);
  if (consumerId) params.set('consumer_id', consumerId);
  const qs = params.toString();
  return request(`/sfu/stats${qs ? `?${qs}` : ''}`);
}

export function connectEvents(): WebSocket {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = getToken();
  const url = `${protocol}//${window.location.host}/api/admin/events`;
  // token via query param for WS (no custom headers in browser WebSocket)
  return new WebSocket(token ? `${url}?token=${encodeURIComponent(token)}` : url);
}