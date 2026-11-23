// ChannelSession (talk-app T11): the level-2 session view -- hold-to-talk
// against one room. Media follows the voice-app pattern: the mic track is
// produced EARLY and the SERVER pauses it unless this peer holds the floor
// (floor_producer_paused) -- so PTT is floor request/release only; the audio
// hardware never lies. Hold countdown comes from the REST floor limits
// (dispatch scope). Receiving others = SfuConsumerClient + audio elements.
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  FloorClient,
  ReplayBuffer,
  SfuConsumerClient,
  SfuMicProducer,
  getToken,
  mixStreams,
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
  const [soundOn, setSoundOn] = useState(true);
  const [volume, setVolume] = useState(0.9);
  const [mutedPeers, setMutedPeers] = useState<string[]>([]);
  const [dispatchScope, setDispatchScope] = useState(false);
  const [replayRecording, setReplayRecording] = useState(false);
  const [replayEnabled, setReplayEnabled] = useState(true);

  const micRef = useRef<SfuMicProducer | null>(null);
  const floorRef = useRef<FloorClient | null>(null);
  const consumerRef = useRef<SfuConsumerClient | null>(null);
  const heldAtRef = useRef<number | null>(null);
  const audioEls = useRef(new Map<string, HTMLAudioElement>());
  const audioCtxRef = useRef<AudioContext | null>(null);
  const volumeRef = useRef(0.9);
  const soundOnRef = useRef(true);
  const selfPeerRef = useRef<string | undefined>(undefined);
  const wasHoldingRef = useRef(false);
  const grantsLatest = useRef<string[]>([]);
  const replayRef = useRef<ReplayBuffer | null>(null);
  const mixRef = useRef<ReturnType<typeof mixStreams>>(null);
  const wsUrl = useMemo(() => wsUrlFor(), []);

  // short synthesized cues (no assets; WebAudio lazily created -- browsers
  // require a user gesture before audio may sound, so cues before the first
  // interaction stay silent by design)
  const cue = (kind: 'granted' | 'lost' | 'denied' | 'join') => {
    if (!soundOnRef.current) return;
    try {
      if (!audioCtxRef.current) audioCtxRef.current = new AudioContext();
      const ctx = audioCtxRef.current;
      if (ctx.state === 'suspended') void ctx.resume();
      const t = ctx.currentTime;
      const tones =
        kind === 'granted'
          ? [[880, 0], [1320, 0.12]]
          : kind === 'lost'
            ? [[660, 0], [440, 0.14]]
            : kind === 'denied'
              ? [[220, 0]]
              : [[700, 0]];
      for (const [freq, at] of tones) {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = kind === 'denied' ? 'square' : 'sine';
        osc.frequency.value = freq;
        gain.gain.setValueAtTime(0.001, t + at);
        gain.gain.linearRampToValueAtTime(0.08, t + at + 0.02);
        gain.gain.exponentialRampToValueAtTime(0.001, t + at + 0.22);
        osc.connect(gain).connect(ctx.destination);
        osc.start(t + at);
        osc.stop(t + at + 0.24);
      }
    } catch {
      /* audio unavailable -- cues are best-effort */
    }
  };

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
        const applyGrants = (g: string[]) => {
          grantsLatest.current = g;
          setGrants(g);
        };
        if (ev.type === 'floor_granted') applyGrants(ev.grants);
        else if (ev.type === 'floor_state') {
          applyGrants(ev.snapshot.grants);
          setMode(ev.snapshot.mode);
          setMutedPeers(ev.snapshot.muted);
          // the queue rides only in dispatch-scoped snapshots (server pruning)
          setDispatchScope(ev.snapshot.queue !== undefined);
        } else if (ev.type === 'floor_idle') applyGrants([]);
        if (ev.type === 'floor_denied') {
          setDenied(ev.reason);
          cue('denied');
        }
        // self-centric cues (refs: the closure predates state updates)
        const nowHolding = selfGranted(grantsLatest.current, selfPeerRef.current);
        if (nowHolding && !wasHoldingRef.current) cue('granted');
        if (!nowHolding && wasHoldingRef.current) cue('lost');
        wasHoldingRef.current = nowHolding;
      },
    });
    floorRef.current = floor;
    mic.onFloorFrame = (data: string) => {
      // roster frames ride the same ws (T6); parse them alongside floor frames
      if (data.includes('"peer_joined"')) cue('join');
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
        mixRef.current?.add(stream);
        if (mic.sessionPeerId) {
          selfPeerRef.current = mic.sessionPeerId;
          setSelfPeer(mic.sessionPeerId);
        }
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
        mixRef.current?.add(stream);
        const key = meta?.producerId ?? `unknown-${Math.random()}`;
        let el = audioEls.current.get(key);
        if (!el) {
          el = document.createElement('audio');
          el.autoplay = true;
          el.volume = volumeRef.current;
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
    // T7 replay rolling buffer (K3/K5/K9): mixes mic + remote audio into one
    // recorder; WIPED on exit by cleanup below (privacy default). Separate
    // from the T9 evidence buffer by design.
    const mix = mixStreams();
    if (mix) {
      mixRef.current = mix;
      const replay = new ReplayBuffer({ seconds: 60 });
      replayRef.current = replay;
      replay.onStateChange = () => setReplayRecording(replay.recording);
      try {
        replay.attach(mix.stream);
      } catch {
        /* MediaRecorder unavailable -- the toggle renders guarded */
      }
    }
    return () => {
      disposed = true;
      mic.close();
      replayRef.current?.detach(); // 退出即清 (K9)
      mixRef.current?.close();
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
      <div className="msr-controls">
        <Button
          size="sm"
          aria-pressed={soundOn}
          onClick={() => {
            soundOnRef.current = !soundOnRef.current;
            setSoundOn(soundOnRef.current);
            // shared preference: the cross-page notification host reads it
            localStorage.setItem('mediaservo_cues_muted', soundOnRef.current ? '0' : '1');
          }}
        >
          {soundOn ? 'Sound on' : 'Sound off'}
        </Button>
        <span className="msr-replay">
          <StatusGlyph
            tone={replayRecording ? 'ok' : 'off'}
            shape={replayRecording ? 'dot' : 'ring'}
            label={replayRecording ? 'replay buffer recording (cleared on exit)' : 'replay buffer off'}
          />
          replay
          <Button
            size="sm"
            aria-pressed={replayEnabled}
            disabled={!mixRef.current}
            disabledReason="no audio source yet"
            onClick={() => {
              const replay = replayRef.current;
              if (!replay) return;
              const next = !replayEnabled;
              setReplayEnabled(next);
              replay.setEnabled(next);
              if (next && mixRef.current) {
                try {
                  replay.attach(mixRef.current.stream);
                } catch {
                  /* MediaRecorder unavailable */
                }
              }
            }}
          >
            {replayEnabled ? 'on' : 'off'}
          </Button>
        </span>
        <label className="msr-volume">
          volume
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={volume}
            onChange={(e) => {
              const v = Number(e.target.value);
              volumeRef.current = v;
              setVolume(v);
              for (const [, el] of audioEls.current) el.volume = v;
            }}
          />
        </label>
      </div>
      <div className="msr-session-grid">
        <div className="msr-session-block">
          <h3>Members</h3>
          <ul className="msr-members">
            {members.map((m) => {
              const isMuted = mutedPeers.includes(m.peer_id);
              const isSelf = m.peer_id === selfPeer;
              return (
                <li key={m.peer_id}>
                  <StatusGlyph
                    tone={m.device_id ? 'ok' : 'off'}
                    shape={m.device_id ? 'dot' : 'ring'}
                    label={m.username ?? m.device_id ?? m.peer_id}
                  />
                  <span>{m.username ?? m.device_id ?? m.peer_id}</span>
                  {m.role && <span className="msr-talk-mode">{m.role}</span>}
                  {isSelf && <span className="msr-talk-mode">you</span>}
                  {isMuted && <span className="msr-muted-tag">muted</span>}
                  {dispatchScope && !isSelf && (
                    <Button
                      size="sm"
                      aria-pressed={isMuted}
                      onClick={() =>
                        micRef.current?.sendFloorCommand({
                          type: 'floor_mute_set',
                          target_peer_id: m.peer_id,
                          muted: !isMuted,
                        })
                      }
                    >
                      {isMuted ? 'unmute' : 'mute'}
                    </Button>
                  )}
                </li>
              );
            })}
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
