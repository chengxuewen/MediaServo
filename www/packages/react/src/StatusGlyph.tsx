export type GlyphTone = 'ok' | 'warn' | 'danger' | 'off' | 'unknown';
export type GlyphShape = 'dot' | 'ring' | 'seg' | 'triangle';

export interface StatusGlyphProps {
  /** Hue channel — never the only channel: always pair with a shape. */
  tone: GlyphTone;
  /** Form channel (design rule: state = hue + shape). dot=held/solid, ring=idle/hollow, seg=queue depth, triangle=alert/unknown. */
  shape?: GlyphShape;
  /** Accessible name, e.g. "held", "queued (2)", "unknown — reconnecting". */
  label: string;
  className?: string;
}

export function StatusGlyph({ tone, shape = 'dot', label, className = '' }: StatusGlyphProps) {
  const cls = ['msr-glyph', `msr-glyph--${tone}`, `msr-glyph--${shape}`, className]
    .filter(Boolean)
    .join(' ');
  return (
    <span className={cls} role="img" aria-label={label} title={label}>
      {shape === 'seg' && (
        <>
          <i />
          <i />
          <i className="is-hollow" />
        </>
      )}
    </span>
  );
}
