import { useCallback, useState } from 'react';
import { VideoPlayer } from '@mediaservo/react';
import { getToken } from '@mediaservo/client';
import StreamTree, { type StreamSelection } from '../components/StreamTree';
import { useStreams } from '../hooks/useStreams';
import { useAdminWS } from '../hooks/useAdminWS';
import { useAuth } from '../hooks/useAuth';

function wsServerUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}

/**
 * Monitor realtime face — slice ③a: device stream tree + single-stream playback.
 * The shared VideoPlayer (packages/react) consumes via SfuConsumerClient; addressing
 * is device room (+ stream label when a registered device exposes one). For a PSK
 * host room with a single producer, consuming the room = that one stream.
 * Multi-stream wall (cap 16) + realtime timeline land in ③b/③c. Recording stays
 * deferred (RECORDING_READY gate, PLAN §8).
 */
export default function Home() {
  const { username, role, canMonitor } = useAuth();
  const { producers, loading, error, refetch } = useStreams();
  const onWsEvent = useCallback(() => { refetch(); }, [refetch]);
  useAdminWS(onWsEvent);
  const [selected, setSelected] = useState<StreamSelection | null>(null);

  return (
    <section className="monitor-home" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ms-space-4)' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ms-space-1)' }}>
        <h1 style={{ fontSize: 'var(--ms-font-size-md)', fontWeight: 'var(--ms-font-weight-semibold)', color: 'var(--ms-color-text)' }}>
          Monitor
        </h1>
        <p role="note" style={{ fontSize: 'var(--ms-font-size-xs)', color: 'var(--ms-color-text-tertiary)', margin: 0 }}>
          Single-stream preview (slice ③a) — signed in as {username ?? 'unknown'}{role ? ` [${role}]` : ''}. {canMonitor ? '' : 'Wall access requires admin/dispatcher. '}Multi-stream wall (cap 16) + realtime timeline land in ③b/③c.
        </p>
      </div>
      <StreamTree producers={producers} loading={loading} error={error} selected={selected} onSelect={setSelected} />
      {selected && (
        <VideoPlayer
          key={selected.producerId}
          roomId={selected.room}
          serverUrl={wsServerUrl()}
          token={getToken() ?? ''}
          variant="modal"
          onClose={() => setSelected(null)}
        />
      )}
    </section>
  );
}