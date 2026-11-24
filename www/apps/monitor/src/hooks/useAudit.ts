import { useState, useEffect, useCallback } from 'react';
import { getAuditEvents, type AuditEventEntry } from '../api/client';

/**
 * Audit ring feed (P3 slice ③c, SB-5/B6 face): GET /api/admin/audit, newest first.
 * Polls 2s; the timeline also refetches on admin WS events for realtime response.
 * Core-in-memory ring (cap 256, not persisted) — honest source per server audit.rs.
 */
export function useAudit() {
  const [events, setEvents] = useState<AuditEventEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetch = useCallback(async () => {
    try {
      const data = await getAuditEvents();
      setEvents([...data.events].sort((a, b) => b.ts - a.ts));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch events');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetch(); }, [fetch]);

  useEffect(() => {
    const interval = setInterval(fetch, 2000);
    return () => clearInterval(interval);
  }, [fetch]);

  return { events, loading, error, refetch: fetch };
}