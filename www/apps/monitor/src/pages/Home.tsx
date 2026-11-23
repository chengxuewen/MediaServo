import { useAuth } from '../hooks/useAuth';

/**
 * P3 slice ① placeholder home — honest banner, no fake data. The realtime
 * surface (device stream tree + video wall cap 16 + realtime timeline) lands in
 * slice ②③ over the shared SDK (SfuConsumerClient) + the admin VideoPlayer
 * pattern. Recording case-library stays deferred (RECORDING_READY gate, PLAN §8).
 */
export default function Home() {
  const { username, role, canMonitor } = useAuth();
  return (
    <section className="monitor-home" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ms-space-4)', maxWidth: 720 }}>
      <h1 style={{ fontSize: 'var(--ms-font-size-md)', fontWeight: 'var(--ms-font-weight-semibold)', color: 'var(--ms-color-text)' }}>
        Monitor
      </h1>
      <div
        role="note"
        style={{
          background: 'var(--ms-color-surface)',
          border: '1px solid var(--ms-color-border)',
          borderRadius: 'var(--ms-radius-md)',
          padding: 'var(--ms-space-4)',
          boxShadow: 'var(--ms-shadow-1)',
          display: 'flex',
          flexDirection: 'column',
          gap: 'var(--ms-space-2)',
        }}
      >
        <p style={{ fontSize: 'var(--ms-font-size-sm)', color: 'var(--ms-color-text-secondary)', margin: 0 }}>
          Thin shell online — login guard + topbar only. The realtime wall (stream tree + video wall + timeline) lands in the next P3 slice.
        </p>
        <p style={{ fontSize: 'var(--ms-font-size-xs)', color: 'var(--ms-color-text-tertiary)', margin: 0 }}>
          Signed in as {username ?? 'unknown'}{role ? ` [${role}]` : ''}. {canMonitor ? 'Wall access: admin/dispatcher admitted.' : 'Wall access requires admin/dispatcher (read-only fallback).'}
        </p>
      </div>
    </section>
  );
}