import { describe, expect, test } from 'vitest';

import { afterClip, dividerScreen, splitFromScreen } from './compare';

const rect = { x: 100, y: 200, w: 400, h: 300 };

describe('compare geometry', () => {
  test('FR-EDT-24 split shows the after image right of the divider, default 50 %', () => {
    expect(afterClip(rect, 'split', 0.5, false)).toEqual({ x: 300, y: 200, w: 200, h: 300 });
    expect(afterClip(rect, 'split', 0, false)).toEqual(rect);
    expect(afterClip(rect, 'split', 1, false).w).toBe(0);
  });

  test('FR-EDT-24 flip shows all of the after image, or none while "before" is held', () => {
    expect(afterClip(rect, 'flip', 0.5, false)).toEqual(rect);
    expect(afterClip(rect, 'flip', 0.5, true).w).toBe(0);
  });

  test('FR-EDT-24 the divider follows the view at any zoom; dragging maps back to 0–1', () => {
    for (const v of [
      { scale: 0.25, tx: 10, ty: -5 },
      { scale: 3, tx: -700, ty: 40 },
    ]) {
      const d = dividerScreen(v, rect, 0.3);
      expect(d.x).toBeCloseTo((100 + 120) * v.scale + v.tx);
      expect(d.h).toBeCloseTo(300 * v.scale);
      expect(splitFromScreen(v, rect, d.x)).toBeCloseTo(0.3);
    }
    expect(splitFromScreen({ scale: 1, tx: 0, ty: 0 }, rect, -1000)).toBe(0);
    expect(splitFromScreen({ scale: 1, tx: 0, ty: 0 }, rect, 1e6)).toBe(1);
  });
});
