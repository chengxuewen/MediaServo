// Cue sounds (P3-3, M4): short Web Audio beeps, no audio assets. Lazy
// AudioContext (autoplay policies: created on the first user gesture).

export type CuePlayer = (kind: 'granted' | 'denied' | 'queued' | 'released') => void;

export function makeCuePlayer(): CuePlayer {
  let ctx: AudioContext | null = null;
  const tone = (freq: number, ms: number, at: number) => {
    if (!ctx) return;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.frequency.value = freq;
    osc.type = 'sine';
    gain.gain.setValueAtTime(0.12, ctx.currentTime + at);
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + at + ms / 1000);
    osc.connect(gain).connect(ctx.destination);
    osc.start(ctx.currentTime + at);
    osc.stop(ctx.currentTime + at + ms / 1000);
  };
  return (kind) => {
    try {
      ctx ??= new AudioContext();
      void ctx.resume();
      switch (kind) {
        case 'granted': tone(880, 90, 0); tone(1320, 120, 0.1); break;
        case 'denied': tone(220, 180, 0); break;
        case 'queued': tone(660, 70, 0); break;
        case 'released': tone(440, 90, 0); break;
      }
    } catch {
      // audio unavailable (headless/autoplay) — cues are best-effort
    }
  };
}
