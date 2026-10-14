import { useEffect, useRef } from 'react';
import { connectEvents, hasToken } from '../api/client';

export function useAdminWS(onEvent: (event: any) => void) {
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!hasToken()) return;

    let ws: WebSocket | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let cancelled = false;
    let attempt = 0;
    const connect = () => {
      if (cancelled) return;
      ws = connectEvents();
      wsRef.current = ws;
      ws.onmessage = (e) => {
        try { onEvent(JSON.parse(e.data)); } catch { /* ignore malformed */ }
      };
      ws.onopen = () => { attempt = 0; };
      ws.onclose = () => {
        if (cancelled) return;
        // W resilience: fixed 5s → exponential backoff 2→30s + jitter (avoids a handshake storm during server restarts)
        attempt++;
        const base = Math.min(2000 * 2 ** (attempt - 1), 30000);
        timer = setTimeout(connect, base * (0.75 + Math.random() * 0.5));
      };
    };

    connect();
    return () => { cancelled = true; if (timer) clearTimeout(timer); ws?.close(); wsRef.current = null; };
  }, [onEvent]);
}
