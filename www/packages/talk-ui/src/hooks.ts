import { useEffect, useState } from 'react';
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
export function useBoardStore(rooms: string[]): BoardBinding {
  const [store] = useState(() => new BoardStore({ rooms }));
  useEffect(() => () => store.close(), [store]);

  const snap = useSyncExternalStore(
    (onChange) => store.subscribe(() => onChange()),
    () => store.snapshot(),
  );
  const send = (roomId: string, cmd: FloorCommand) => store.sendCommand(roomId, cmd);
  return { snap, send };
}
