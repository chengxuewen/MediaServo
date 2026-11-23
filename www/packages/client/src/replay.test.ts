import { beforeEach, describe, expect, it } from 'vitest';
import { ReplayBuffer, type ReplayRecorder } from './replay';

class FakeRecorder implements ReplayRecorder {
  static instances: FakeRecorder[] = [];
  ondataavailable: ((e: { data: Blob }) => void) | null = null;
  started = false;
  timeslice: number | undefined;
  constructor(
    public stream: unknown,
    public opts?: object,
  ) {
    FakeRecorder.instances.push(this);
  }
  start(ts?: number) {
    this.started = true;
    this.timeslice = ts;
  }
  stop() {
    this.started = false;
  }
  emit(size = 4) {
    this.ondataavailable?.({ data: new Blob([new Uint8Array(size)]) });
  }
}

const fakeStream = {} as MediaStream;

describe('ReplayBuffer (T7 / K3+K5+K9)', () => {
  let now = 0;
  const clock = () => now;

  const make = (seconds = 60) =>
    new ReplayBuffer({
      seconds,
      timesliceMs: 2000,
      now: clock,
      recorderFactory: (s, ts) => new FakeRecorder(s, { ts }),
    });

  beforeEach(() => {
    FakeRecorder.instances = [];
    now = 0;
  });

  it('attach starts the recorder and chunks accumulate', () => {
    const buf = make();
    buf.attach(fakeStream);
    const rec = FakeRecorder.instances.at(-1)!;
    expect(rec.started).toBe(true);
    expect(buf.recording).toBe(true);
    now = 2000;
    rec.emit();
    now = 4000;
    rec.emit();
    expect(buf.bufferedMs()).toBe(4000);
  });

  it('ring trims to the configured window (drops oldest)', () => {
    const buf = make(6); // 6 s window
    buf.attach(fakeStream);
    const rec = FakeRecorder.instances.at(-1)!;
    for (let i = 1; i <= 10; i++) {
      now = i * 2000;
      rec.emit();
    }
    expect(buf.bufferedMs()).toBeLessThanOrEqual(6000);
    expect(buf.bufferedMs()).toBeGreaterThan(0);
  });

  it('exportClip picks only the recent chunks; empty buffer -> null', () => {
    const buf = make();
    expect(buf.exportClip(30_000)).toBeNull();
    buf.attach(fakeStream);
    const rec = FakeRecorder.instances.at(-1)!;
    now = 2000;
    rec.emit(8);
    now = 10_000;
    rec.emit(4);
    const clip = buf.exportClip(5_000); // last 5 s -> only the second chunk
    expect(clip).not.toBeNull();
    expect(clip!.size).toBe(4);
    const all = buf.exportClip(60_000);
    expect(all!.size).toBe(12);
  });

  it('detach stops the recorder and WIPES the buffer (退出即清)', () => {
    const buf = make();
    buf.attach(fakeStream);
    const rec = FakeRecorder.instances.at(-1)!;
    now = 2000;
    rec.emit();
    buf.detach();
    expect(rec.started).toBe(false);
    expect(buf.recording).toBe(false);
    expect(buf.bufferedMs()).toBe(0);
  });

  it('setEnabled(false) stops + wipes; attach is a no-op while disabled', () => {
    const buf = make();
    buf.attach(fakeStream);
    const rec = FakeRecorder.instances.at(-1)!;
    now = 2000;
    rec.emit();
    buf.setEnabled(false);
    expect(rec.started).toBe(false);
    expect(buf.bufferedMs()).toBe(0);
    const before = FakeRecorder.instances.length;
    buf.attach(fakeStream);
    expect(FakeRecorder.instances.length).toBe(before); // no new recorder
    expect(buf.recording).toBe(false);
  });

  it('onStateChange fires on attach/detach', () => {
    const buf = make();
    let hits = 0;
    buf.onStateChange = () => {
      hits += 1;
    };
    buf.attach(fakeStream);
    buf.detach();
    expect(hits).toBe(2);
  });
});
