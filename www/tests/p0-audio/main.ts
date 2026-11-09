import { SfuConsumerClient, SfuMicProducer, login } from '@mediaservo/client';

type Metrics = { bitrate?: number; [k: string]: unknown };

interface P0State {
  role: string;
  status: string;
  produced?: boolean;
  track?: boolean;
  metrics?: Metrics;
  error?: string | null;
}

declare global {
  interface Window {
    __p0: P0State;
    __p0Close?: () => void;
  }
}

const params = new URLSearchParams(location.search);
const role = params.get('role') ?? 'sub';
const room = params.get('room') ?? 'audio-p0';
const username = params.get('user') ?? 'dispatcher';
const password = params.get('pass') ?? 'dispatch123';
const wsBase = location.origin.replace(/^http/, 'ws');

const statusEl = document.getElementById('status');
document.getElementById('role')!.textContent = role;
window.__p0 = { role, status: 'boot', error: null };

function setStatus(s: string): void {
  window.__p0.status = s;
  if (statusEl) statusEl.textContent = s;
}

async function main(): Promise<void> {
  try {
    const { token } = await login(username, password);
    if (role === 'pub') {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true, video: false });
      const producer = new SfuMicProducer(wsBase, room, token);
      await producer.produceAudio(stream.getAudioTracks()[0]);
      window.__p0.produced = true;
      setStatus('produced');
      window.__p0Close = () => {
        producer.close();
        stream.getTracks().forEach((t) => t.stop());
      };
      return;
    }

    const audio = document.createElement('audio');
    audio.autoplay = true;
    audio.id = 'p0-audio';
    document.body.appendChild(audio);

    const consumer = new SfuConsumerClient(wsBase, room, token, {
      onTrack: (s) => {
        audio.srcObject = s;
        void audio.play().catch(() => {});
        window.__p0.track = true;
        setStatus('track');
      },
      onStatus: (s) => setStatus(s),
      onMetrics: (m) => {
        window.__p0.metrics = m as Metrics;
      },
    });
    await consumer.connect();
    await consumer.startPlay();
    setStatus('connected');
    window.__p0Close = () => consumer.close();
  } catch (e) {
    window.__p0.error = String((e as Error)?.message ?? e);
    setStatus('error');
  }
}

void main();
