import { SfuConsumerClient, SfuMicProducer, login } from '@mediaservo/client';

type Metrics = { bitrate?: number; [k: string]: unknown };
type Stat = {
  type: string;
  kind?: string;
  bytesSent?: number;
  bytesReceived?: number;
  packetsSent?: number;
  packetsReceived?: number;
  ssrc?: number;
  roundTripTime?: number;
  jitter?: number;
};

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
    __p0Stats?: () => Promise<Stat[]>;
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

// O24: expose the per-stream byte counters so the runner can assert real RTP flow
// (PIT-105 lesson: "wired" is not "flowing" — only byte counts prove it).
let statsProbe: (() => Promise<Stat[]>) | null = null;

async function gatherStats(
  transport: { getStats(): Promise<unknown> },
  dir: 'out' | 'in',
): Promise<Stat[]> {
  const report = (await transport.getStats()) as unknown as {
    values?: () => Iterable<Stat>;
  } & Record<string, Stat>;
  // RTCStatsReport is a Map — Object.values() on it is always empty.
  const rows =
    typeof report?.values === 'function' ? Array.from(report.values()) : Object.values(report ?? {});
  // 'out' also carries remote-inbound-rtp: the RTT/jitter source (latency baseline).
  const want =
    dir === 'out' ? ['outbound-rtp', 'remote-inbound-rtp'] : ['inbound-rtp'];
  return rows.filter((s) => s && typeof s === 'object' && want.includes(s.type));
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
      const pTransport = (
        producer as unknown as { transport: { getStats(): Promise<unknown> } | null }
      ).transport;
      if (pTransport) {
        statsProbe = () => gatherStats(pTransport, 'out');
        window.__p0Stats = statsProbe;
      }
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

    let consumerRef: SfuConsumerClient | null = null;
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
    consumerRef = consumer;
    await consumer.connect();
    await consumer.startPlay();
    setStatus('connected');
    const rTransport = (
      consumer as unknown as { transport: { getStats(): Promise<unknown> } | null }
    ).transport;
    if (rTransport) {
      statsProbe = () => gatherStats(rTransport, 'in');
      window.__p0Stats = statsProbe;
    }
    window.__p0Close = () => consumer.close();
  } catch (e) {
    window.__p0.error = String((e as Error)?.message ?? e);
    setStatus('error');
  }
}

void main();
