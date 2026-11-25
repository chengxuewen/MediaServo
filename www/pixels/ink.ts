import { PNG } from 'pngjs';
import type { Locator } from '@playwright/test';

/** Rendered-pixel ink box of an element (bright pixels only). This measures what the
 *  user actually sees — DOM box centres hide baseline offsets between different font
 *  sizes, and canvas font metrics mis-model the ascent (seen 2026-11-25: 5-6px lie). */
export type InkBox = {
  top: number; bottom: number; left: number; right: number;
  cx: number; cy: number; width: number; height: number;
};

export async function inkBox(
  target: Locator,
  opts: { threshold?: number; xRange?: [number, number] } = {},
): Promise<InkBox> {
  const threshold = opts.threshold ?? 150;
  const shot = await target.screenshot();
  const png = PNG.sync.read(Buffer.from(shot));
  const { width, height, data } = png;
  const lum = (x: number, y: number) => {
    const i = (y * width + x) * 4;
    return 0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2];
  };
  const [x0, x1] = opts.xRange ?? [0, width];
  let top = Infinity, bottom = -1, left = Infinity, right = -1;
  for (let y = 0; y < height; y++) {
    for (let x = x0; x < x1; x++) {
      if (lum(x, y) > threshold) {
        if (y < top) top = y;
        if (y > bottom) bottom = y;
        if (x < left) left = x;
        if (x > right) right = x;
      }
    }
  }
  if (bottom < 0) throw new Error('inkBox: no ink pixels found (raise contrast or lower threshold)');
  return {
    top, bottom, left, right,
    cx: (left + right) / 2, cy: (top + bottom) / 2,
    width: right - left + 1, height: bottom - top + 1,
  };
}
/** Measure several x-bands INSIDE one screenshot of `target` (same coordinate space),
 *  so baselines/insets of elements in different containers can be compared safely. */
export async function inkBoxes(
  target: Locator,
  ranges: Array<[number, number]>,
  opts: { threshold?: number } = {},
): Promise<InkBox[]> {
  const threshold = opts.threshold ?? 150;
  const shot = await target.screenshot();
  const png = PNG.sync.read(Buffer.from(shot));
  const { width, height, data } = png;
  const lum = (x: number, y: number) => {
    const i = (y * width + x) * 4;
    return 0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2];
  };
  return ranges.map(([x0, x1]) => {
    const a = Math.max(0, Math.min(width - 1, Math.round(x0)));
    const b = Math.max(a + 1, Math.min(width, Math.round(x1)));
    let top = Infinity, bottom = -1, left = Infinity, right = -1;
    for (let y = 0; y < height; y++) {
      for (let x = a; x < b; x++) {
        if (lum(x, y) > threshold) {
          if (y < top) top = y;
          if (y > bottom) bottom = y;
          if (x < left) left = x;
          if (x > right) right = x;
        }
      }
    }
    if (bottom < 0) return { top: -1, bottom: -1, left: -1, right: -1, cx: -1, cy: -1, width: 0, height: 0 };
    return { top, bottom, left, right, cx: (left + right) / 2, cy: (top + bottom) / 2, width: right - left + 1, height: bottom - top + 1 };
  });
}
