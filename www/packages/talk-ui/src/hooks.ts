import { useMemo, useState } from 'react';
import { useSyncExternalStore } from 'react';
import { BoardStore, type BoardSnapshot, type FloorCommand } from '@mediaservo/client';

export interface BoardBinding {
  snap: BoardSnapshot;
  send: (roomId: string, cmd: FloorCommand) => void;
}

/**
 * React binding over the framework-agnostic BoardStore (D4: logic lives in
 * `@mediaservo/client`, React only renders).
 *
 * The store lives for the lifetime of the mount and is closed on unmount.
 * `rooms` must be reference-stable across renders (memoize / keep in state) —
 * a new array identity creates a new store.
 */
// Module-level store cache, keyed by the room set. BoardStore is SHARED app
// state (I2: one multi-room aggregator per room set), so stores are cached and
// reused across mounts -- React 18 StrictMode double-invokes effects (mount ->
// cleanup -> mount) and a per-mount close() would permanently kill the sockets
// (closedByUser suppresses reconnects). No unmount close: the cache lives for
// the page lifetime; tests construct BoardStore directly instead.
const storeCache = new Map<string, BoardStore>();

export function useBoardStore(rooms: string[]): BoardBinding {
  const [key] = useState(() => rooms.join('\u0000'));
  const store = useMemo(() => {
    let s = storeCache.get(key);
    if (!s) {
      s = new BoardStore({ rooms });
      storeCache.set(key, s);
    }
    return s;
  }, [key]);

  const snap = useSyncExternalStore(
    (onChange) => store.subscribe(() => onChange()),
    () => store.snapshot(),
  );
  const send = (roomId: string, cmd: FloorCommand) => store.sendCommand(roomId, cmd);
  return { snap, send };
}
