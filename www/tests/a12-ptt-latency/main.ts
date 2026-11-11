// A12 PTT latency harness page (voice-web-app P3 note): one page, two roles.
//
// pub: synthesizes a CONTINUOUS 440 Hz oscillator track (not the fake device —
// its beep/silence duty cycle would corrupt the edge: the first loud sample
// must equal the grant-driven forwarding resume, not the tone's next beep).
// With R6 media-first (non-granted producers are paused server-side, sfu.rs
// pause_producer/resume_producer) silence is real zero-RTP, so the listener
// edge timestamps exactly the floor grant landing in the media plane.
//
// sub: meters RMS on the remote stream via AnalyserNode and timestamps the
// first sample above threshold after the driver arms it.
//
// Clock: both pages use Date.now() on the same host — wall clock is shared,
// sub-millisecond skew, negligible against the 350 ms budget.
import {
  SfuConsumerClient,
  SfuMicProducer,
  FloorClient,
  login,
} from '@mediaservo/client';

interface A12State {
  role: string;
  status: string;
  produced?: boolean;
  track?: boolean;
  granted?: boolean;
  lastEvent?: string;
  t0?: number;
  grantedAt?: number | null;
  edge?: number | null;
  edgeRms?: number;
  armRms?: number;
  rms?: number;
  error?: string | null;
}

declare global {
  interface Window {
    __a12: A12State;
    __a12Press?: () => void;
    __a12Release?: () => void;
    __a12Arm?: () => void;
  }
}

// ~-34 dBFS. A paused producer sends NO RTP, so idle RMS is float noise near 0.
const RMS_THRESHOLD = 0.02;

const params = new URLSearchParams(location.search);
const role = params.get('role') ?? 'sub';
const room = params.get('room') ?? 'talk-a12';
const username = params.get('user') ?? 'operator';
const password = params.get('pass') ?? 'operator123';
const wsBase = location.origin.replace(/^http/, 'ws');

const statusEl = document.getElementById('status');
document.getElementById('role')!.textContent = role;
window.__a12 = { role, status: 'boot', error: null };

function setStatus(s: string): void {
  window.__a12.status = s;
  if (statusEl) statusEl.textContent = s;
}

async function main(): Promise<void> {
  try {
    const { token } = await login(username, password);

    if (role === 'pub') {
      const actx = new AudioContext();
      const osc = actx.createOscillator();
      osc.frequency.value = 440;
      const dest = actx.createMediaStreamDestination();
      osc.connect(dest);
      osc.start();
      const producer = new SfuMicProducer(wsBase, room, token);
      await producer.produceAudio(dest.stream.getAudioTracks()[0]);
      const floor = new FloorClient({
        send: (cmd) => producer.sendFloorCommand(cmd),
        roomId: room,
        onEvent: (ev) => {
          window.__a12.lastEvent = ev.type;
          if (ev.type === 'floor_granted') {
            // grants name the SESSION peer (O26-surfaced identity rule)
            const self = producer.sessionPeerId;
            if (!self || ev.grants.includes(self)) {
              window.__a12.grantedAt = Date.now();
              window.__a12.granted = true;
            }
          } else if (ev.type === 'floor_idle' || ev.type === 'floor_taken') {
            window.__a12.granted = false;
          }
        },
      });
      producer.onFloorFrame = (data) => floor.handleWire(data);
      floor.resync();
      window.__a12Press = () => {
        window.__a12.t0 = Date.now();
        window.__a12.grantedAt = null;
        floor.request();
      };
      window.__a12Release = () => floor.release();
      window.__a12.produced = true;
      setStatus('produced');
      return;
    }

    const audio = document.createElement('audio');
    audio.autoplay = true;
    audio.muted = true; // meter only — no speaker path needed in headless
    document.body.appendChild(audio);

    const consumer = new SfuConsumerClient(wsBase, room, token, {
      onTrack: (s) => {
        audio.srcObject = s;
        void audio.play().catch(() => {});
        const actx = new AudioContext();
        const src = actx.createMediaStreamSource(s);
        const analyser = actx.createAnalyser();
        analyser.fftSize = 1024;
        src.connect(analyser);
        const buf = new Float32Array(analyser.fftSize);
        let armed = false;
        window.__a12Arm = () => {
          window.__a12.edge = null;
          window.__a12.edgeRms = undefined;
          window.__a12.armRms = window.__a12.rms;
          armed = true;
        };
        setInterval(() => {
          analyser.getFloatTimeDomainData(buf);
          let sum = 0;
          for (const v of buf) sum += v * v;
          const rms = Math.sqrt(sum / buf.length);
          window.__a12.rms = rms;
          if (armed && rms > RMS_THRESHOLD) {
            window.__a12.edge = Date.now();
            window.__a12.edgeRms = rms;
            armed = false;
          }
        }, 3);
        window.__a12.track = true;
        setStatus('track');
      },
      onStatus: (s) => setStatus(s),
    });
    await consumer.connect();
    await consumer.startPlay();
    setStatus('connected');
  } catch (e) {
    window.__a12.error = String((e as Error)?.message ?? e);
    setStatus('error');
  }
}

void main();
