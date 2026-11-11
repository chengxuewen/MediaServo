import { useEffect, useRef, useState } from 'react';
import {
  FloorClient,
  SfuConsumerClient,
  login,
  getToken,
  type FloorServerEvent,
  type StreamMeta,
} from '@mediaservo/client';
import { initialFloorUi, reduceFloorUi, talkPosture, type FloorUiState } from './state';

/** Dispatch console (P3-3): subscribes to every stream in the room (O21 M1),
 *  shows the floor panel (grants/queue/mode) and sends dispatcher commands
 *  (mode_set/mute_set) on the same consumer connection. */
export function DispatchPage({ room, wsUrl }: { room: string; wsUrl: string }) {
  const [token, setToken] = useState<string | null>(getToken());
  const [user, setUser] = useState('');
  const [pass, setPass] = useState('');
  const [phase, setPhase] = useState<'login' | 'connecting' | 'live'>('login');
  const [state, setState] = useState<FloorUiState>(initialFloorUi());
  const [streams, setStreams] = useState<Map<string, { stream: MediaStream; meta: StreamMeta }>>(new Map());

  const clientRef = useRef<SfuConsumerClient | null>(null);
  const audioEls = useRef<Map<string, HTMLAudioElement>>(new Map());
  const stateRef = useRef<FloorUiState>(state);

  useEffect(() => { stateRef.current = state; }, [state]);
  useEffect(() => () => { clientRef.current?.close(); }, []);
  // reload path (O26 B3/B9): a stored token connects automatically
  useEffect(() => {
    if (phase === 'login' && token && !user && !pass) void connect();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- run once on mount
  }, []);

  const connect = async () => {
    let t = token;
    if (!t || !t.length) {
      const res = await login(user, pass);
      t = res.token;
      localStorage.setItem('mediaservo_admin_token', t);
      setToken(t);
    }
    setPhase('connecting');
    const floor = new FloorClient({
      send: (cmd) => clientRef.current?.sendFloorCommand(cmd),
      roomId: room,
      onEvent: (ev: FloorServerEvent) => setState((s) => reduceFloorUi(s, ev)),
    });
    const client = new SfuConsumerClient(wsUrl, room, t, {
      onTrack: (stream, meta) => {
        const key = meta?.producerId ?? `unknown-${Math.random()}`;
        setStreams((m) => new Map(m).set(key, { stream, meta: meta ?? { producerId: key } }));
      },
      onStatus: (st) => { if (st === 'error') setPhase('connecting'); },
      onMetrics: () => { /* dispatch console: metrics not surfaced */ },
    }, {
      // O21 M1: empty label set = consume EVERY producer in the room; floor
      // frames delegate to FloorClient (P3-3 seam).
      onFloorFrame: (data) => floor.handleWire(data),
    });
    clientRef.current = client;
    await client.connect();
    await client.startPlay();
    setPhase('live');
  };

  useEffect(() => {
    // one <audio> element per stream, autoplay attached
    for (const [key, entry] of streams) {
      let el = audioEls.current.get(key);
      if (!el) {
        el = document.createElement('audio');
        el.autoplay = true;
        el.srcObject = entry.stream;
        document.body.appendChild(el);
        el.style.display = 'none';
        audioEls.current.set(key, el);
      }
    }
    return () => {
      for (const [key, el] of audioEls.current) {
        if (!streams.has(key)) { el.remove(); audioEls.current.delete(key); }
      }
    };
  }, [streams]);

  const setMode = (mode: string) => clientRef.current?.sendFloorCommand({ type: 'floor_mode_set', mode });
  const toggleMute = (peer: string, muted: boolean) =>
    clientRef.current?.sendFloorCommand({ type: 'floor_mute_set', target_peer_id: peer, muted });

  if (phase !== 'live') {
    return (
      <div>
        <h1>Dispatch — {room}</h1>
        {phase === 'connecting' ? <p>Connecting…</p> : (
          <form onSubmit={(e) => { e.preventDefault(); void connect(); }}>
            <input placeholder="user" value={user} onChange={(e) => setUser(e.target.value)} />
            <input placeholder="password" type="password" value={pass} onChange={(e) => setPass(e.target.value)} />
            <button type="submit">Enter</button>
          </form>
        )}
      </div>
    );
  }

  const selfPeer = undefined; // dispatch view: self id not needed for posture
  const posture = talkPosture({ ...state, selfPeer });

  return (
    <div>
      <h1>Dispatch — {room}</h1>
      <p>Streams: {streams.size} | Mode: {state.mode ?? 'unknown'} | Floor: {state.grants.length ? state.grants.join(', ') : 'idle'}</p>
      <p>{posture === 'granted' ? 'floor active' : ''}</p>
      <h2>Floor panel</h2>
      <p>
        {['exclusive', 'hybrid', 'open'].map((m) => (
          <button key={m} onClick={() => setMode(m)} style={{ marginRight: 8 }}>{m}</button>
        ))}
      </p>
      {state.grants.map((g) => (
        <button key={g} onClick={() => toggleMute(g, !state.muted.includes(g))} style={{ marginRight: 8 }}>
          {state.muted.includes(g) ? `unmute ${g}` : `mute ${g}`}
        </button>
      ))}
      {state.muted
        .filter((m) => !state.grants.includes(m))
        .map((m) => (
          <button key={m} onClick={() => toggleMute(m, false)} style={{ marginRight: 8 }}>
            unmute {m}
          </button>
        ))}
      {state.queue.length > 0 && (
        <p>
          Queue:{' '}
          {state.queue.map((q) => (
            <span key={q.peer} style={{ marginRight: 8 }}>
              {q.peer}({q.priority})
              <button
                onClick={() => {
                  const client = clientRef.current;
                  if (client) {
                    // O18: bump +5 (ceiling 100 enforced server-side; private
                    // position update + fresh snapshot come back on the wire).
                    void client.sendFloorCommand({
                      type: 'floor_set_priority',
                      target_peer_id: q.peer,
                      priority: Math.min(100, q.priority + 5),
                    });
                  }
                }}
              >
                +prio
              </button>
            </span>
          ))}
        </p>
      )}
    </div>
  );
}
