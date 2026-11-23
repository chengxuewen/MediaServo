// ChannelSession (talk-app T11): the level-2 session view -- hold-to-talk
// against one room. Media follows the voice-app pattern: the mic track is
// produced EARLY and the SERVER pauses it unless this peer holds the floor
// (floor_producer_paused) -- so PTT is floor request/release only; the audio
// hardware never lies. Hold countdown comes from the REST floor limits
// (dispatch scope). Receiving others = SfuConsumerClient + audio elements.
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  FloorClient,
  SfuConsumerClient,
  SfuMicProducer,
  getToken,
  type FloorServerEvent,
} from '@mediaservo/client';
import { Button, StatusGlyph, type GlyphShape, type GlyphTone } from '@mediaservo/react';

export interface ChannelSessionProps {
  roomId: string;
  /** Max hold seconds from the REST floor limits (dispatch scope). */
  holdSecs?: number;
  /** Roster presence from the board's BoardStore (peer_id -> presence). */
  roster?: Record<string, { peer_id: string; username?: string; role?: string; device_id?: string }>;
  onBack?: () => void;
  className?: string;
}

function wsUrlFor(): string {
  const origin = typeof location !== 'undefined' ? location.origin : '';
  return `${origin.replace(/^http/, 'ws')}/ws`;
}

const selfGranted = (grants: string[], self: string | undefined) =>
  Boolean(self && grants.includes(self));

export function ChannelSession({
  roomId,
  holdSecs = 30,
  onBack,
  className = '',
}: ChannelSessionProps) {
  const [selfPeer, setSelfPeer] = useState<string | undefined>(undefined);
  const [grants, setGrants] = useState<string[]>([]);
  const [mode, setMode] = useState<string>('exclusive');
  const [denied, setDenied] = useState<string | null>(null);
  const [members, setMembers] = useState<
    { peer_id: string; username?: string; role?: string; device_id?: string }[]
  >([]);
  const [now, setNow] = useState(() => Date.now());
  const [live, setLive] = useState(false);

  const micRef = useRef<SfuMicProducer | null>(null);
  const floorRef = useRef<FloorClient | null>(null);
  const consumerRef = useRef<SfuConsumerClient | null>(null);
  const heldAtRef = useRef<number | null>(null);
  const audioEls = useRef(new Map<string, HTMLAudioElement>());
  const wsUrl = useMemo(() => wsUrlFor(), []);

  const holding = selfGranted(grants, selfPeer);
  const holdLeft =
    holding && heldAtRef.current !== null
      ? Math.max(0, Math.ceil(holdSecs - (now - heldAtRef.current) / 1000))
      : null;

  // 1s tick for the hold countdown
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(t);
  }, []);

  // media + floor wiring (voice FieldPage pattern: produce early, the SERVER
  // pauses the track unless this peer holds the floor; floor frames ride the
  // producer ws, the consumer ws delegates to the same FloorClient).
  useEffect(() => {
    const token = getToken() ?? '';
    let disposed = false;
    const mic = new SfuMicProducer(wsUrl, roomId, token);
    micRef.current = mic;
    const floor = new FloorClient({
      send: (cmd) => mic.sendFloorCommand(cmd),
      roomId,
      onEvent: (ev: FloorServerEvent) => {
        if (ev.type === 'floor_granted') setGrants(ev.grants);
        else if (ev.type === 'floor_state') {
          setGrants(ev.snapshot.grants);
          setMode(ev.snapshot.mode);
        } else if (ev.type === 'floor_idle') setGrants([]);
        if (ev.type === 'floor_denied') setDenied(ev.reason);
      },
    });
    floorRef.current = floor;
    mic.onFloorFrame = (data: string) => {
      // roster frames ride the same ws (T6); parse them alongside floor frames
      if (data.includes('"room_peers"')) {
        try {
          const m = JSON.parse(data) as {
            peers?: { peer_id: string; username?: string; role?: string; device_id?: string }[];
          };
          if (Array.isArray(m.peers)) setMembers(m.peers);
        } catch {
          /* ignore */
        }
      }
      floor.handleWire(data);
    };
    void (async () => {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        if (disposed) return;
        await mic.produceAudio(stream.getAudioTracks()[0]);
        floor.resync();
        if (mic.sessionPeerId) setSelfPeer(mic.sessionPeerId);
        setLive(true);
      } catch (e) {
        // honest gate: mic unavailable -- floor CONTROL still works (the
        // server grants, then its no-producer grace auto-revokes shortly after;
        // the cue explains why audio never flows). Keep the session live.
        setDenied(`microphone unavailable: ${e instanceof Error ? e.message : String(e)}`);
        setLive(true);
      }
    })();
    const consumer = new SfuConsumerClient(
      wsUrl,
      roomId,
      token,
      {
      onTrack: (stream, meta) => {
        const key = meta?.producerId ?? `unknown-${Math.random()}`;
        let el = audioEls.current.get(key);
        if (!el) {
          el = document.createElement('audio');
          el.autoplay = true;
          el.srcObject = stream;
          document.body.appendChild(el);
          el.style.display = 'none';
          audioEls.current.set(key, el);
        }
      },
      onStatus: () => {},
      onMetrics: () => {},
    },
      {
        // floor frames delegate to the same FloorClient (P3-3 seam, voice
        // DispatchPage pattern) -- generation filtering dedupes both feeds.
        onFloorFrame: (data: string) => floor.handleWire(data),
      },
    );
    consumerRef.current = consumer;
    void (async () => {
      try {
        await consumer.connect();
        await consumer.startPlay();
      } catch {
        /* receive side degrades gracefully; PTT is independent */
      }
    })();
    return () => {
      disposed = true;
      mic.close();
      for (const [, el] of audioEls.current) el.remove();
      audioEls.current.clear();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- roomId fixed per view
  }, [roomId, wsUrl]);

  const press = () => {
    heldAtRef.current = Date.now();
    setDenied(null);
    floorRef.current?.request();
  };
  const releasePress = () => {
    heldAtRef.current = null;
    floorRef.current?.release();
  };

  // keyboard PTT: hold Space (keydown w/o repeat), release on keyup; Esc = back
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !e.repeat && live) {
        e.preventDefault();
        press();
      }
      if (e.code === 'Escape' && onBack) onBack();
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === 'Space' && live) {
        e.preventDefault();
        releasePress();
      }
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- press/release close over refs
  }, [live, onBack]);

  const tone: GlyphTone = holding ? 'ok' : denied ? 'danger' : 'off';
  const shape: GlyphShape = holding ? 'dot' : denied ? 'triangle' : 'ring';

  return (
    <div className={['msr-session', className].filter(Boolean).join(' ')}>
      <div className="msr-session-head">
        {onBack && (
          <Button size="sm" onClick={onBack} aria-label="back to the board">
            &larr; Board
          </Button>
        )}
        <strong className="msr-talk-name">{roomId}</strong>
        <StatusGlyph
          tone={tone}
          shape={shape}
          label={holding ? `held by you (${holdLeft ?? holdSecs}s left)` : denied ?? 'idle'}
        />
        <span className="msr-talk-mode">{mode}</span>
      </div>
      {denied && (
        <p className="msr-error" role="alert">
          {denied}
        </p>
      )}
      <button
        type="button"
        className={holding ? 'msr-ptt msr-ptt--live' : 'msr-ptt'}
        aria-pressed={holding}
        onPointerDown={press}
        onPointerUp={releasePress}
        onPointerLeave={holding ? releasePress : undefined}
        disabled={!live}
        aria-disabled={!live}
        data-reason={live ? undefined : 'connecting -- mic and floor are being set up'}
      >
        {holding ? `TALKING - ${holdLeft ?? holdSecs}s` : 'HOLD TO TALK'}
      </button>
      <p className="msr-hint">Hold the button (or hold Space). Release to give up the floor.</p>
      <div className="msr-session-grid">
        <div className="msr-session-block">
          <h3>Members</h3>
          <ul className="msr-members">
            {members.map((m) => (
              <li key={m.peer_id}>
                <StatusGlyph
                  tone={m.device_id ? 'ok' : 'off'}
                  shape={m.device_id ? 'dot' : 'ring'}
                  label={m.username ?? m.device_id ?? m.peer_id}
                />
                <span>{m.username ?? m.device_id ?? m.peer_id}</span>
                {m.role && <span className="msr-talk-mode">{m.role}</span>}
              </li>
            ))}
            {!members.length && <li className="msr-hint">roster arrives with the join snapshot...</li>}
          </ul>
        </div>
        <div className="msr-session-block">
          <h3>Now holding</h3>
          <p className="msr-talk-state">{holding ? 'you' : grants.join(' + ') || '-'}</p>
        </div>
      </div>
    </div>
  );
}
