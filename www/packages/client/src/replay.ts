// Replay rolling buffer (talk-app T7 = design K3=A + K5=A + K9, §7.5).
//
// A SHORT rolling audio buffer for "what did they just say?" replay. Privacy
// defaults per K5/K9: cleared on session exit (detach), channel-level toggle
// (off = stop + wipe), and SEPARATE from the evidence buffer (T9 -- event
// triggers with consent, retained + audited; this module must never feed it).
//
// MediaRecorder consumes the live stream; chunks land in a ring trimmed to
// `seconds`. The session mixes mic + remote streams via mixStreams() so a
// single recorder captures the whole conversation.
//
// Injectable clock + recorder factory for tests (no real MediaRecorder in
// node).

export interface ReplayChunk {
  /** start/end times (ms, injected clock). */
  t0: number;
  t1: number;
  blob: Blob;
}

export interface ReplayOptions {
  /** Rolling window length (default 60 s). */
  seconds?: number;
  /** MediaRecorder timeslice (default 2000 ms). */
  timesliceMs?: number;
  recorderFactory?: (stream: MediaStream, timesliceMs: number) => ReplayRecorder;
  now?: () => number;
}

/** Minimal MediaRecorder surface the buffer needs (real one satisfies it). */
export interface ReplayRecorder {
  start(timeslice?: number): void;
  stop(): void;
  ondataavailable: ((e: { data: Blob }) => void) | null;
  state?: string;
}

export class ReplayBuffer {
  private readonly seconds: number;
  private readonly timesliceMs: number;
  private readonly factory: (stream: MediaStream, timesliceMs: number) => ReplayRecorder;
  private readonly now: () => number;
  private chunks: ReplayChunk[] = [];
  private recorder: ReplayRecorder | null = null;
  /** Channel-level switch (K16 category 3: single click, rebuildable). */
  enabled = true;
  /** Fires on every recording-state change (drives the "recording" indicator). */
  onStateChange: (() => void) | null = null;

  constructor(opts: ReplayOptions = {}) {
    this.seconds = opts.seconds ?? 60;
    this.timesliceMs = opts.timesliceMs ?? 2000;
    this.factory =
      opts.recorderFactory ??
      ((_stream, _ts) => {
        const R = (globalThis as unknown as { MediaRecorder?: new (s: MediaStream, o?: object) => ReplayRecorder })
          .MediaRecorder;
        if (!R) throw new Error('replay: MediaRecorder unavailable');
        return new R(_stream, { mimeType: 'audio/webm' });
      });
    this.now = opts.now ?? (() => Date.now());
  }

  get recording(): boolean {
    return this.recorder !== null && this.enabled;
  }

  /** Start consuming a stream (mic+remote composite). No-op when disabled. */
  attach(stream: MediaStream): void {
    if (this.recorder) this.detach(); // re-attach = wipe first (privacy)
    if (!this.enabled) return;
    const rec = this.factory(stream, this.timesliceMs);
    let t0 = this.now();
    rec.ondataavailable = (e) => {
      const t1 = this.now();
      if (e.data && e.data.size > 0) {
        this.chunks.push({ t0, t1, blob: e.data });
        this.trim();
      }
      t0 = t1;
    };
    rec.start(this.timesliceMs);
    this.recorder = rec;
    this.onStateChange?.();
  }

  /** Stop recording and WIPE the buffer (session exit -- K9 privacy default). */
  detach(): void {
    if (this.recorder) {
      try {
        this.recorder.stop();
      } catch {
        /* already stopped */
      }
      this.recorder = null;
    }
    this.chunks = [];
    this.onStateChange?.();
  }

  /** Consent withdrawal / manual clear (keeps recording if still attached). */
  clear(): void {
    this.chunks = [];
  }

  /** Channel-level switch OFF = stop + wipe; ON only re-enables attach(). */
  setEnabled(on: boolean): void {
    if (this.enabled === on) return;
    this.enabled = on;
    if (!on) this.detach();
    else this.onStateChange?.();
  }

  /** Total buffered audio (ms). */
  bufferedMs(): number {
    return this.chunks.reduce((n, c) => n + (c.t1 - c.t0), 0);
  }

  /** Assemble the last N ms into one blob (null when nothing overlaps). */
  exportClip(lastMs: number): Blob | null {
    const floor = this.now() - lastMs;
    const picked = this.chunks.filter((c) => c.t1 > floor);
    if (!picked.length) return null;
    return new Blob(
      picked.map((c) => c.blob),
      { type: 'audio/webm' },
    );
  }

  private trim(): void {
    const limit = this.seconds * 1000;
    while (this.chunks.length > 1) {
      const total = this.bufferedMs();
      if (total <= limit) break;
      this.chunks.shift();
    }
  }
}

/**
 * Mix several live streams (mic + remote producers) into ONE stream so a
 * single recorder captures the whole conversation. WebAudio only; browsers
 * without AudioContext return null (callers degrade honestly).
 */
export interface MixedStream {
  stream: MediaStream;
  add(stream: MediaStream): void;
  remove(stream: MediaStream): void;
  close(): void;
}

export function mixStreams(): MixedStream | null {
  const AC = (globalThis as unknown as { AudioContext?: new () => AudioContext }).AudioContext;
  if (!AC) return null;
  const ctx = new AC();
  const dest = ctx.createMediaStreamDestination();
  const inputs = new Map<MediaStream, MediaStreamAudioSourceNode>();
  return {
    stream: dest.stream,
    add(stream) {
      if (inputs.has(stream)) return;
      try {
        const src = ctx.createMediaStreamSource(stream);
        src.connect(dest);
        inputs.set(stream, src);
      } catch {
        /* source rejected (no audio tracks) -- skip */
      }
    },
    remove(stream) {
      const src = inputs.get(stream);
      if (!src) return;
      try {
        src.disconnect(dest);
      } catch {
        /* already disconnected */
      }
      inputs.delete(stream);
    },
    close() {
      for (const src of inputs.values()) {
        try {
          src.disconnect(dest);
        } catch {
          /* ignore */
        }
      }
      inputs.clear();
      void ctx.close();
    },
  };
}
