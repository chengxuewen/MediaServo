import { useDevices } from '../hooks/useDevices';
import { useAdminWS } from '../hooks/useAdminWS';
import { useAuth } from '../hooks/useAuth';
import StatsCard from '../components/StatsCard';
import StatusBadge from '../components/StatusBadge';
import { getStats, deleteRoom } from '../api/client';
import { useState, useEffect, useCallback } from 'react';
import type { StatsResponse, DeviceSnapshot, StreamSnapshot } from '../api/client';
import StreamDetail from '../components/StreamDetail';
import VideoPlayer from '../components/VideoPlayer';
import './Dashboard.css';
import { ChevronRight, Play, Video, Eye } from 'lucide-react';

// 列数控制（play-layout-stats）：默认 3、最多 4，localStorage 持久化 + 脏值防御
const MAX_PLAYING = 4;
const PLAY_COLS_KEY = "mediaservo_play_cols";
const MAX_COLS = 4;
const DEFAULT_COLS = 3;

export default function Dashboard() {
  const { devices, loading, error, refetch: refetchDevices } = useDevices();
  const { isAdmin } = useAuth();
  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [expanded, setExpanded] = useState(new Set());
  const [selectedStream, setSelectedStream] = useState<{ deviceId: string; stream: StreamSnapshot } | null>(null);
  // multi-stream P3: 勾选集（roomId = device_stream，与 deleteRoom 约定一致）+ 播放中列表
  const [selectedRooms, setSelectedRooms] = useState<Set<string>>(new Set());
  const [playingRooms, setPlayingRooms] = useState<string[]>([]);
  // tile 双击→大窗播放（产品方案②）：独立 modal 实例，关闭即弃
  const [modalRoom, setModalRoom] = useState<string | null>(null);
  const [cols, setCols] = useState<number>(() => {
    let v: number;
    try { v = Number(localStorage.getItem(PLAY_COLS_KEY)); } catch { v = NaN; }
    return Number.isInteger(v) && v >= 1 && v <= MAX_COLS ? v : DEFAULT_COLS;
  });
  const applyCols = (n: number) => {
    setCols(n);
    try { localStorage.setItem(PLAY_COLS_KEY, String(n)); } catch { /* 无痕降级（隐私模式） */ }
  };

  const fetchStats = useCallback(async () => {
    try { setStats(await getStats()); } catch { /* ignore */ }
  }, []);

  useEffect(() => { fetchStats(); }, [fetchStats]);

  useAdminWS(() => {
    fetchStats();
    // 列表秒级刷新: admin 事件（StreamCreate/StreamDestroy 等）→ 立即重拉设备/流快照。
    refetchDevices();
  });

  const toggle = (id: string) => {
    setExpanded(prev => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  };

  const toggleSelect = (roomId: string) => {
    setSelectedRooms(prev => {
      const next = new Set(prev);
      next.has(roomId) ? next.delete(roomId) : next.add(roomId);
      return next;
    });
  };

  const roomIdOf = (deviceId: string, streamId: string) => `${deviceId}_${streamId}`;

  // 播放勾选集（空勾选 = 播放全部流，上限 MAX_PLAYING）
  const playSelected = (deviceId: string) => {
    const device = devices.find(d => d.device_id === deviceId);
    if (!device) return;
    let rooms: string[];
    if (selectedRooms.size > 0) {
      rooms = device.streams
        .map(s => roomIdOf(deviceId, s.stream_id))
        .filter(r => selectedRooms.has(r));
    } else {
      rooms = device.streams.map(s => roomIdOf(deviceId, s.stream_id));
    }
    if (rooms.length === 0) return;
    setPlayingRooms(prev => {
      const merged = [...prev];
      for (const r of rooms) if (!merged.includes(r) && merged.length < MAX_PLAYING) merged.push(r);
      return merged;
    });
  };

  const toggleAllDevice = (device: DeviceSnapshot, checked: boolean) => {
    setSelectedRooms(prev => {
      const next = new Set(prev);
      for (const s of device.streams) {
        const r = roomIdOf(device.device_id, s.stream_id);
        checked ? next.add(r) : next.delete(r);
      }
      return next;
    });
  };

  const totalStreams = devices.reduce((s, d) => s + d.streams.length, 0);
  const totalConsumers = devices.reduce((s, d) => s + d.streams.reduce((c, st) => c + st.consumers.length, 0), 0);

  if (loading) return <div className="loading">Loading...</div>;
  if (error) return <div className="error">{error}</div>;

  return (
    <div className="dashboard">
      <div className="stats-bar">
        <StatsCard label="Devices" value={devices.length} />
        <StatsCard label="Streams" value={totalStreams} />
        <StatsCard label="Consumers" value={totalConsumers} />
        <StatsCard label="Peers" value={stats?.total_peers ?? '-'} />
      </div>

      <h2 className="section-title">Active Devices</h2>
      {devices.length === 0 ? (
        <p className="empty">No active devices</p>
      ) : (
        <div className="device-list">
          {devices.map((device) => (
            <DeviceGroup
              key={device.device_id}
              device={device}
              canManage={isAdmin}
              expanded={expanded.has(device.device_id)}
              selectedRooms={selectedRooms}
              onToggle={() => toggle(device.device_id)}
              onSelectStream={(stream) => setSelectedStream({ deviceId: device.device_id, stream })}
              onToggleSelect={(roomId) => toggleSelect(roomId)}
              onToggleAll={(checked) => toggleAllDevice(device, checked)}
              onPlaySelected={() => playSelected(device.device_id)}
              onCloseRoom={(roomId) => setPlayingRooms(prev => prev.filter(r => r !== roomId))}
            />
          ))}
        </div>
      )}
      {selectedStream && (
        <StreamDetail
          deviceId={selectedStream.deviceId}
          streamId={selectedStream.stream.stream_id}
          consumers={selectedStream.stream.consumers}
          canManage={isAdmin}
          onClose={() => setSelectedStream(null)}
        />
      )}
      {playingRooms.length > 0 && (
        <>
          <div className="video-toolbar">
            <span className="vt-label">列数</span>
            {[1, 2, 3, 4].map((n) => (
              <button key={n} className={n === cols ? "vt-btn active" : "vt-btn"} aria-pressed={n === cols} onClick={() => applyCols(n)}>
                {n}
              </button>
            ))}
          </div>
          <div className="video-grid" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}>
            {playingRooms.map((roomId) => (
              <VideoPlayer
                key={roomId}
                roomId={roomId}
                serverUrl={wsServerUrl()}
                token={localStorage.getItem('mediaservo_admin_token') || ''}
                variant="tile"
                onExpand={() => setModalRoom(roomId)}
                onClose={() => { setPlayingRooms(prev => prev.filter(r => r !== roomId)); setModalRoom(prev => (prev === roomId ? null : prev)); }}
              />
            ))}
          </div>
        </>
      )}
      {modalRoom && (
        <VideoPlayer
          key={`modal-${modalRoom}`}
          roomId={modalRoom}
          serverUrl={wsServerUrl()}
          token={localStorage.getItem('mediaservo_admin_token') || ''}
          variant="modal"
          onClose={() => setModalRoom(null)}
        />
      )}
    </div>
  );
}

function wsServerUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}

function DeviceGroup({ device, canManage, expanded, selectedRooms, onToggle, onSelectStream, onToggleSelect, onToggleAll, onPlaySelected, onCloseRoom }: {
  device: DeviceSnapshot; canManage: boolean; expanded: boolean;
  selectedRooms: Set<string>;
  onToggle: () => void; onSelectStream: (stream: StreamSnapshot) => void;
  onToggleSelect: (roomId: string) => void; onToggleAll: (checked: boolean) => void;
  onPlaySelected: () => void; onCloseRoom: (roomId: string) => void;
}) {
  const status = device.streams.length > 0 ? 'online' : 'offline';
  const deviceAllSelected = device.streams.length > 0 && device.streams.every(s => selectedRooms.has(`${device.device_id}_${s.stream_id}`));
  const anySelected = device.streams.some(s => selectedRooms.has(`${device.device_id}_${s.stream_id}`));

  return (
    <div className="device-group">
      <div className="device-header" onClick={onToggle}>
        <span className={`tree-icon ${expanded ? 'expanded' : ''}`}><ChevronRight size={12} /></span>
        <span className="device-name">{device.device_id}</span>
        <StatusBadge status={status} />
        <span className="device-stream-count">{device.streams.length} streams</span>
        <button className="btn-play" onClick={(e) => { e.stopPropagation(); onPlaySelected(); }}>
          <Play size={12} /> {anySelected ? 'Watch Selected' : 'Play All'}
        </button>
        {canManage && (
          <label className="stream-select-all" onClick={(e) => e.stopPropagation()}>
            <input
              type="checkbox"
              checked={deviceAllSelected}
              onChange={(e) => onToggleAll(e.target.checked)}
              title="Select all streams"
            />
            <span>Select All</span>
          </label>
        )}
      </div>
      {expanded && (
        <div className="stream-list">
          {device.streams.map((stream) => {
            const roomId = `${device.device_id}_${stream.stream_id}`;
            const playing = selectedRooms.has(roomId);
            return (
              <div key={stream.stream_id} className="stream-item" onClick={() => onSelectStream(stream)} style={{ cursor: 'pointer' }}>
                <span className="stream-dot" style={{ background: stream.online ? '#27ae60' : '#95a5a6' }} title={stream.online ? 'online' : 'offline'} />
                <span className="stream-name"><Video size={14} /> {stream.stream_id}</span>
                <span className="consumer-count">{stream.consumers.length} viewers</span>
                <div className="consumer-list">
                  {stream.consumers.map((c) => (
                    <span key={c.peer_id} className="consumer-tag"><Eye size={12} /> {c.peer_id}</span>
                  ))}
                </div>
                <label className="stream-select" onClick={(e) => e.stopPropagation()}>
                  <input type="checkbox" checked={playing} onChange={() => onToggleSelect(roomId)} title="Select for joint viewing" />
                </label>
                {canManage && <button className="btn-sm" onClick={(e) => { e.stopPropagation(); deleteRoom(roomId); }}>Close</button>}
                {playing && <button className="btn-sm btn-outline" onClick={(e) => { e.stopPropagation(); onCloseRoom(roomId); }}>Stop</button>}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
