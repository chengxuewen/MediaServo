import { useCallback, useEffect, useState } from 'react';
import { getSfuRooms, getSfuStats, type SfuRoom, type SfuStats as SfuStatsT } from '../api/client';
import './Audio.css';
import { Mic } from 'lucide-react';

interface RoomStats {
  producerStats: SfuStatsT[];
  consumerStats: SfuStatsT[];
  error?: string;
}

/** 拉取单房间全部 producer/consumer 的 SfuStats（H2 协议管理面 REST 路径）。 */
async function fetchRoomStats(room: SfuRoom): Promise<RoomStats> {
  const [producerStats, consumerStats] = await Promise.all([
    Promise.all(room.producer_ids.map((id) => getSfuStats(id, undefined))),
    Promise.all(room.consumer_ids.map((id) => getSfuStats(undefined, id))),
  ]);
  return { producerStats, consumerStats };
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024).toFixed(0)} KB`;
}

export default function Audio() {
  const [rooms, setRooms] = useState<SfuRoom[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [statsByRoom, setStatsByRoom] = useState<Record<string, RoomStats>>({});
  const [statsLoading, setStatsLoading] = useState<Set<string>>(new Set());

  const fetchRooms = useCallback(async () => {
    try {
      const data = await getSfuRooms();
      setRooms(data.rooms.filter((r) => r.audio));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch audio rooms');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchRooms(); }, [fetchRooms]);
  // ponytail: 5s 轮询（与 useDevices 同节奏）; WS 事件合并留后续
  useEffect(() => {
    const interval = setInterval(fetchRooms, 5000);
    return () => clearInterval(interval);
  }, [fetchRooms]);

  const refreshStats = useCallback(async (roomId: string) => {
    const room = rooms.find((r) => r.room_id === roomId);
    if (!room || room.producer_ids.length + room.consumer_ids.length === 0) return;
    setStatsLoading((prev) => new Set(prev).add(roomId));
    try {
      const stats = await fetchRoomStats(room);
      setStatsByRoom((prev) => ({ ...prev, [roomId]: stats }));
    } catch (e) {
      setStatsByRoom((prev) => ({ ...prev, [roomId]: { producerStats: [], consumerStats: [], error: e instanceof Error ? e.message : 'stats failed' } }));
    } finally {
      setStatsLoading((prev) => { const next = new Set(prev); next.delete(roomId); return next; });
    }
  }, [rooms]);

  // 首次加载后自动拉一轮 stats（房间存在 producer/consumer 时）
  useEffect(() => {
    if (!loading) rooms.forEach((r) => refreshStats(r.room_id));
  }, [loading]); // eslint-disable-line react-hooks/exhaustive-deps

  const totalParticipants = rooms.reduce((s, r) => s + r.participants, 0);
  const totalProducers = rooms.reduce((s, r) => s + r.producers, 0);
  const totalConsumers = rooms.reduce((s, r) => s + r.consumers, 0);

  if (loading) return <div className="loading">Loading...</div>;
  if (error && rooms.length === 0) return <div className="error">{error}</div>;

  return (
    <div className="audio">
      <div className="stats-bar">
        <div className="stats-card">
          <span className="stats-label">Audio Rooms</span>
          <span className="stats-value">{rooms.length}</span>
        </div>
        <div className="stats-card">
          <span className="stats-label">Participants</span>
          <span className="stats-value">{totalParticipants}</span>
        </div>
        <div className="stats-card">
          <span className="stats-label">Producers</span>
          <span className="stats-value">{totalProducers}</span>
        </div>
        <div className="stats-card">
          <span className="stats-label">Consumers</span>
          <span className="stats-value">{totalConsumers}</span>
        </div>
      </div>

      <h2 className="section-title">Audio Conference Rooms</h2>
      {rooms.length === 0 ? (
        <p className="empty">No active audio rooms</p>
      ) : (
        <div className="audio-room-list">
          {rooms.map((room) => {
            const stats = statsByRoom[room.room_id];
            return (
              <div key={room.room_id} className="audio-room">
                <div className="audio-room-header">
                  <span className="audio-room-name"><Mic size={14} /> {room.room_id.replace(/^audio-/, '')}</span>
                  <span className="audio-room-id">{room.room_id}</span>
                  <span className="audio-room-counts">
                    {room.participants} participants · {room.producers} producers · {room.consumers} consumers
                  </span>
                  <button
                    className="btn-sm btn-stats"
                    onClick={() => refreshStats(room.room_id)}
                    disabled={statsLoading.has(room.room_id)}
                  >
                    {statsLoading.has(room.room_id) ? 'Refreshing...' : 'Refresh Stats'}
                  </button>
                </div>
                {stats?.error && <p className="audio-stats-error">{stats.error}</p>}
                {stats && (stats.producerStats.length > 0 || stats.consumerStats.length > 0) && (
                  <div className="audio-stats">
                    {stats.producerStats.map((s) => (
                      <div key={`p-${s.producer_id}`} className="audio-stat-row">
                        <span className="audio-stat-label">↑ producer {s.producer_id?.slice(0, 8)}</span>
                        <span>{formatBytes(s.byte_count)}</span>
                        <span>{s.packet_count} pkts</span>
                        <span className="audio-stat-score">score {s.score}</span>
                      </div>
                    ))}
                    {stats.consumerStats.map((s) => (
                      <div key={`c-${s.consumer_id}`} className="audio-stat-row">
                        <span className="audio-stat-label">↓ consumer {s.consumer_id?.slice(0, 8)}</span>
                        <span>{formatBytes(s.byte_count)}</span>
                        <span>{s.packet_count} pkts</span>
                        <span className="audio-stat-score">score {s.score}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
