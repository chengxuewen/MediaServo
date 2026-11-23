// Multi-room scan listening (talk-app T8, design K1 high-value trio):
// keep up to MON_CAP rooms audible at once. The FOCUS room is the session's
// own room (its consumer lives in ChannelSession); this hook adds listen-only
// consumers for the OTHER rooms. PTT always targets the focus room by
// construction (the session's floor/mic wiring never changes target).
import { useEffect, useRef } from 'react';
import { SfuConsumerClient, getToken } from '@mediaservo/client';

/** Concurrency cap for monitored rooms (focus included). */
export const MON_CAP = 4;

interface ScanHandle {
  close?: () => void;
}

export function useRoomScan(
  allRooms: string[],
  focusRoomId: string,
  enabled: boolean,
  wsUrl: string,
): void {
  const clientsRef = useRef<Map<string, ScanHandle>>(new Map());
  const elsRef = useRef<Map<string, HTMLAudioElement>>(new Map());

  useEffect(() => {
    const slots = Math.max(0, MON_CAP - 1); // focus takes one slot
    const wanted = enabled ? allRooms.filter((r) => r !== focusRoomId).slice(0, slots) : [];

    for (const room of wanted) {
      if (clientsRef.current.has(room)) continue;
      const client = new SfuConsumerClient(
        wsUrl,
        room,
        getToken() ?? '',
        {
          onTrack: (stream) => {
            let el = elsRef.current.get(room);
            if (!el) {
              el = document.createElement('audio');
              el.autoplay = true;
              el.volume = 0.9;
              document.body.appendChild(el);
              elsRef.current.set(room, el);
            }
            el.srcObject = stream;
          },
          onStatus: () => {},
          onMetrics: () => {},
        },
      );
      void (async () => {
        try {
          await client.connect();
          await client.startPlay();
        } catch {
          /* scan room degrades silently -- focus audio is unaffected */
        }
      })();
      clientsRef.current.set(room, client as unknown as ScanHandle);
    }

    for (const [room, client] of clientsRef.current) {
      if (!wanted.includes(room)) {
        client.close?.();
        clientsRef.current.delete(room);
        const el = elsRef.current.get(room);
        if (el) {
          el.remove();
          elsRef.current.delete(room);
        }
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- roomId/wsUrl stable per session
  }, [allRooms, focusRoomId, enabled, wsUrl]);

  useEffect(
    () => () => {
      for (const [, client] of clientsRef.current) client.close?.();
      clientsRef.current.clear();
      for (const [, el] of elsRef.current) el.remove();
      elsRef.current.clear();
    },
    [],
  );
}
