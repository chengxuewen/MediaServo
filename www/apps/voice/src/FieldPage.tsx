import { useEffect, useMemo, useRef, useState } from 'react';
import {
  FloorClient,
  SfuMicProducer,
  login,
  getToken,
  type FloorServerEvent,
} from '@mediaservo/client';
import { initialFloorUi, reduceFloorUi, cueFor, talkPosture, type FloorUiState } from './state';
import { makeCuePlayer } from './cues';

/** Field cockpit (P3-3, M4): mic + floor on ONE connection (same peer
 *  session), hold-to-talk + optional lock, cue sounds. */
export function FieldPage({ room, wsUrl }: { room: string; wsUrl: string }) {
  const [token, setToken] = useState<string | null>(getToken());
  const [user, setUser] = useState('');
  const [pass, setPass] = useState('');
  const [phase, setPhase] = useState<'login' | 'connecting' | 'live'>('login');
  const [state, setState] = useState<FloorUiState>(initialFloorUi());
  const [locked, setLocked] = useState(false);

  const micRef = useRef<SfuMicProducer | null>(null);
  const floorRef = useRef<FloorClient | null>(null);
  const stateRef = useRef<FloorUiState>(state);
  const playCue = useMemo(() => makeCuePlayer(), []);

  useEffect(() => { stateRef.current = state; }, [state]);
  useEffect(() => () => { micRef.current?.close(); }, []);
  // reload path (O26 B9 resync): a stored token connects automatically
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
    const producer = new SfuMicProducer(wsUrl, room, t);
    micRef.current = producer;
    const floor = new FloorClient({
      send: (cmd) => producer.sendFloorCommand(cmd),
      roomId: room,
      onEvent: (ev: FloorServerEvent) => {
        const cue = cueFor(stateRef.current, ev);
        if (cue) playCue(cue);
        setState((s) => reduceFloorUi(s, ev));
      },
    });
    floorRef.current = floor;
    producer.onFloorFrame = (data) => floor.handleWire(data);
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    await producer.produceAudio(stream.getAudioTracks()[0]);
    floor.resync(); // late-joiner replay: pull the current floor truth
    if (producer.sessionPeerId) {
      // the floor identity is the SESSION peer (server-assigned at room_join),
      // not the mic transport peer -- grants name this id (O26-surfaced).
      const fresh = initialFloorUi(producer.sessionPeerId);
      stateRef.current = fresh;
      setState(fresh);
    }
    setPhase('live');
  };

  const press = () => {
    if (!floorRef.current) return;
    // hold-to-talk = QUEUE (no preempt): preempting would deny a lower-priority
    // peer outright (PreemptPriority) instead of the M4 wait-your-turn posture.
    // Preempt is an explicit dispatcher-side action, not a field-page default.
    if (locked) {
      // lock posture: one click toggles talk; release only via another click
      const grantedNow = stateRef.current.selfPeer
        && stateRef.current.grants.includes(stateRef.current.selfPeer);
      if (grantedNow) floorRef.current.release();
      else floorRef.current.request();
    } else {
      floorRef.current.request();
    }
  };
  const releasePress = () => {
    if (!locked && floorRef.current) floorRef.current.release();
  };

  const posture = talkPosture(state);
  const selfGranted = posture === 'granted';

  if (phase !== 'live') {
    return (
      <div>
        <h1>Field — {room}</h1>
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

  return (
    <div>
      <h1>Field — {room}</h1>
      <p>
        {selfGranted
          ? 'You have the floor'
          : posture === 'queued'
            ? 'Queued — waiting for the floor'
            : state.deniedReason
              ? `Denied: ${state.deniedReason}`
              : 'Tap and hold to talk'}
      </p>
      <button
        style={{
          width: 220, height: 220, borderRadius: '50%', fontSize: 24,
          background: selfGranted
            ? '#1d7d46'
            : posture === 'queued'
              ? '#b8860b'
              : posture === 'denied'
                ? '#7d1d1d'
                : '#2a3138',
          color: '#fff', border: '2px solid #444',
          touchAction: 'none', userSelect: 'none',
        }}
        onPointerDown={press}
        onPointerUp={releasePress}
        onPointerLeave={releasePress}
      >TALK</button>
      <p>
        <label>
          <input type="checkbox" checked={locked} onChange={(e) => setLocked(e.target.checked)} />
          Lock (click to toggle instead of hold)
        </label>
      </p>
      {state.queue.length > 0 && (
        <p>Queue: {state.queue.map((q) => q.peer).join(' → ')}</p>
      )}
    </div>
  );
}
