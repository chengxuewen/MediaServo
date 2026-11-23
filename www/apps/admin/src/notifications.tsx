// Cross-page floor notifications (talk-app T11 = design 7.2, K15=A).
// The BoardStore lives in a module cache (page lifetime), so floor frames
// keep arriving on every page -- this host turns the three K15 events
// (granted-you / queued-to-you / external emergency [no server source yet])
// into expiring topbar chips. In-page (/intercom) events never produce chips
// (the session surfaces them inline); the nav badge is a pure held count.
//
// The store is constructed ONLY after a token exists (BoardStore throws
// board:no-token by design) -- the provider itself mounts app-wide, including
// the login page, so the store-bound part lives in StoreBridge.
import { createContext, useContext, useEffect, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { getToken, type BoardSnapshot } from '@mediaservo/client';
import { useBoardStore } from '@mediaservo/talk-ui';
import { listManagedRooms } from './api/client';

export interface FloorNotifChip {
  id: string;
  roomId: string;
  kind: 'granted' | 'queued';
  label: string;
  createdAt: number;
  expireAt: number;
}

interface FloorNotifCtx {
  chips: FloorNotifChip[];
  heldCount: number;
  dismissChip: (id: string) => void;
}

const Ctx = createContext<FloorNotifCtx>({ chips: [], heldCount: 0, dismissChip: () => {} });

const HELD_TTL_MS = 30_000; // server default max_hold_secs
const QUEUED_TTL_MS = 60_000; // K15 chip expiry for queued-to-you
const MAX_CHIPS = 3;
const cuesMuted = () => localStorage.getItem('mediaservo_cues_muted') === '1';

function playCue(kind: 'granted' | 'queued') {
  if (cuesMuted()) return;
  try {
    const ctx = new AudioContext();
    if (ctx.state === 'suspended') void ctx.resume();
    const t = ctx.currentTime;
    const tones = kind === 'granted' ? [[880, 0], [1320, 0.12]] : [[700, 0], [880, 0.12]];
    for (const [freq, at] of tones) {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0.001, t + at);
      gain.gain.linearRampToValueAtTime(0.06, t + at + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.001, t + at + 0.2);
      osc.connect(gain).connect(ctx.destination);
      osc.start(t + at);
      osc.stop(t + at + 0.22);
    }
  } catch {
    /* best-effort */
  }
}

/** Store-bound bridge: only rendered after a token exists. */
function StoreBridge({
  rooms,
  onNewChips,
  onHeldCount,
}: {
  rooms: string[];
  onNewChips: (chips: FloorNotifChip[]) => void;
  onHeldCount: (n: number) => void;
}) {
  const location = useLocation();
  const lastEvents = useRef<Map<string, BoardSnapshot['rooms'][string]['lastEvent']>>(new Map());
  const onIntercom = location.pathname.startsWith('/intercom');
  const { snap } = useBoardStore(rooms);

  useEffect(() => {
    const nextChips: FloorNotifChip[] = [];
    for (const roomId of rooms) {
      const r = snap.rooms[roomId];
      if (!r) continue;
      const prev = lastEvents.current.get(roomId);
      if (prev === r.lastEvent) continue;
      lastEvents.current.set(roomId, r.lastEvent);
      if (onIntercom) continue; // K15: in-page events surface inline, no chips
      const ev = r.lastEvent;
      if (!ev) continue;
      const self = r.peerId ?? undefined;
      if (ev.type === 'floor_granted' && self && ev.grants.includes(self)) {
        nextChips.push({
          id: `${roomId}-granted-${ev.generation}`,
          roomId,
          kind: 'granted',
          label: `Floor granted to you in ${roomId}`,
          createdAt: Date.now(),
          expireAt: Date.now() + HELD_TTL_MS,
        });
      } else if (ev.type === 'floor_queued' && self) {
        nextChips.push({
          id: `${roomId}-queued-${ev.generation}`,
          roomId,
          kind: 'queued',
          label: `You are queued (#${ev.position}) in ${roomId}`,
          createdAt: Date.now(),
          expireAt: Date.now() + QUEUED_TTL_MS,
        });
      }
    }
    if (nextChips.length) onNewChips(nextChips);

    const held = rooms.reduce((n, roomId) => {
      const r = snap.rooms[roomId];
      return (
        n +
        (r && r.peerId && r.lastEvent?.type === 'floor_granted' && r.lastEvent.grants.includes(r.peerId)
          ? 1
          : 0)
      );
    }, 0);
    onHeldCount(held);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- drives off the store snapshot
  }, [snap, rooms, onIntercom]);

  return null;
}

export function FloorNotifProvider({ children }: { children: React.ReactNode }) {
  const [rooms, setRooms] = useState<string[]>([]);
  const [chips, setChips] = useState<FloorNotifChip[]>([]);
  const [heldCount, setHeldCount] = useState(0);
  const [tokenReady, setTokenReady] = useState(() => Boolean(getToken()));
  const location = useLocation();

  // the app may mount on /login; pick the token up after login
  useEffect(() => {
    if (!tokenReady && getToken()) setTokenReady(true);
  }, [location.pathname, tokenReady]);

  useEffect(() => {
    if (!tokenReady) return;
    let alive = true;
    listManagedRooms()
      .then((r) => {
        if (alive) setRooms(r.rooms.filter((x) => x.kind === 'talk').map((x) => x.id));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [tokenReady]);

  // expiry sweep (1s)
  useEffect(() => {
    const t = setInterval(() => {
      const now = Date.now();
      setChips((cs) => {
        const kept = cs.filter((c) => c.expireAt > now);
        return kept.length === cs.length ? cs : kept;
      });
    }, 1000);
    return () => clearInterval(t);
  }, []);

  const mergeChips = (nextChips: FloorNotifChip[]) => {
    for (const c of nextChips) playCue(c.kind);
    setChips((cs) => {
      const merged = [...cs.filter((c) => !nextChips.some((n) => n.id === c.id)), ...nextChips];
      // per-room dedup: newest chip wins per (roomId, kind)
      const seen = new Set<string>();
      const deduped = merged.filter((c) => {
        const k = `${c.roomId}:${c.kind}`;
        if (seen.has(k)) return false;
        seen.add(k);
        return true;
      });
      // cap 3: drop the oldest beyond the cap
      return deduped.slice(-MAX_CHIPS);
    });
  };

  const dismissChip = (id: string) => setChips((cs) => cs.filter((c) => c.id !== id));

  // e2e/devtools hook (dev builds only): inject a chip without media
  useEffect(() => {
    const isDev = (import.meta as unknown as { env?: { DEV?: boolean } }).env?.DEV !== false;
    if (!isDev) return;
    (
      window as unknown as { __floorNotifInject?: (room: string, kind: 'granted' | 'queued') => void }
    ).__floorNotifInject = (room, kind) => {
      // K15 in-page discipline applies to injections too (real events are
      // filtered in StoreBridge; the hook must behave identically)
      if (location.pathname.startsWith('/intercom')) return;
      const now = Date.now();
      setChips((cs) => [
        ...cs,
        {
          id: `${room}-${kind}-inject-${now}`,
          roomId: room,
          kind,
          label: kind === 'granted' ? `Floor granted to you in ${room}` : `You are queued in ${room}`,
          createdAt: now,
          expireAt: now + (kind === 'granted' ? HELD_TTL_MS : QUEUED_TTL_MS),
        },
      ]);
    };
  }, []);

  const roomsKey = rooms.join('\u0000');
  return (
    <Ctx.Provider value={{ chips, heldCount, dismissChip }}>
      {tokenReady && rooms.length > 0 && (
        <StoreBridge key={roomsKey} rooms={rooms} onNewChips={mergeChips} onHeldCount={setHeldCount} />
      )}
      {children}
    </Ctx.Provider>
  );
}

export function useFloorNotifs(): FloorNotifCtx {
  return useContext(Ctx);
}

/** Topbar chip stack (rendered inside the header, before the version chip). */
export function FloorNotifChips() {
  const { chips, dismissChip } = useContext(Ctx);
  const navigate = useNavigate();
  if (!chips.length) return null;
  return (
    <div id="floorNotif" className="notif-stack" aria-live="polite">
      {chips.map((c) => (
        <button
          key={c.id}
          type="button"
          className={`notif-chip notif-chip--${c.kind}`}
          onClick={() => {
            dismissChip(c.id);
            navigate(`/intercom/${c.roomId}`);
          }}
        >
          {c.label}
        </button>
      ))}
    </div>
  );
}
