import { useState, useEffect, useCallback } from 'react';
import { getSfuStatsList, type SfuStreamStatRow } from '../api/client';

/**
 * Live producer list (P3 slice ③a, B5 corrected contract): GET /api/admin/sfu/stats
 * full list, "in the table = alive" (a row exists only with a selected ICE tuple).
 * Polls 5s + WS-event refresh (mirrors admin useDevices posture). The tree groups
 * these producers by device room; the producer uuid feeds single-stream metrics
 * (?producer_id) in a later slice. NOT sourced from /rooms.devices[].streams —
 * that registry list is empty for a PSK host (the B1/S1-L2 blocker admin hit).
 */
export function useStreams() {
  const [producers, setProducers] = useState<SfuStreamStatRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetch = useCallback(async () => {
    try {
      const data = await getSfuStatsList();
      setProducers(data.streams.filter((s) => s.role === 'producer'));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch streams');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetch(); }, [fetch]);

  useEffect(() => {
    const interval = setInterval(fetch, 5000);
    return () => clearInterval(interval);
  }, [fetch]);

  return { producers, loading, error, refetch: fetch };
}