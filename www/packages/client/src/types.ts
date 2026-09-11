// ── server wire 类型（P0/D270-R1 自 apps/admin api/client.ts 提纯——逐字迁移，零逻辑面）──

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
// device-enroll §5.4: 待批准队列（验签过、未入册；内存表重启即清）
export interface PendingDevice { device_id: string; public_key: string; first_seen_ms: number; verified: boolean; }
export interface PendingDeviceListResponse { pending: PendingDevice[]; count: number; }
export interface PendingDeviceApproved { device_id: string; public_key: string; name: string | null; note: string; }
export type AccountRole = 'viewer' | 'operator' | 'admin' | 'dispatcher';
export interface AdminAccount { username: string; role: string; vehicles: string[]; }
export interface AdminAccountListResponse { accounts: AdminAccount[]; count: number; }
export interface AdminAccountCreated { created: string; }
export interface AdminAccountUpdated { updated: string; }
export interface AdminAccountDeleted { deleted: string; }

// ── PSK 管理（psk-admin-management — admin-only 端点）──────────────────────

export interface PskResponse {
  psk: string;
  hint: string;
}
