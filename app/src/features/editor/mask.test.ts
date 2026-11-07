import { describe, expect, test } from 'vitest';

import { dabsAlong, dabSpacing, emptyMask, maskHash, remap, stamp, stampAlpha } from './mask';
import { MaskController, type MaskView } from './MaskController';
import { applyDrag, moveRect, rectFromPoints, resizeRect } from './region';
import { mockRegionFit, RegionError } from './regionFit';
import type { RegionState } from './store';
import { UndoStack } from './undo';

const image = { w: 2000, h: 1500 };

describe('brush stamp', () => {
  test('FR-EDT-15 alpha is 1 inside h·r and falls linearly to 0 at r', () => {
    expect(stampAlpha(0, 10, 0.5)).toBe(1);
    expect(stampAlpha(5, 10, 0.5)).toBe(1);
    expect(stampAlpha(7.5, 10, 0.5)).toBeCloseTo(0.5);
    expect(stampAlpha(10, 10, 0.5)).toBe(0);
    expect(stampAlpha(5, 10, 0)).toBeCloseTo(0.5);
    expect(dabSpacing(2)).toBe(1);
    expect(dabSpacing(40)).toBe(10);
  });

  test('FR-EDT-15 a 100 % hardness dab gives 255 inside and 0 outside', () => {
    const m = emptyMask({ x: 0, y: 0, w: 40, h: 40 });
    stamp(m, 20, 20, 10, 1, 'paint');
    for (let y = 0; y < 40; y++)
      for (let x = 0; x < 40; x++) {
        const d = Math.hypot(x + 0.5 - 20, y + 0.5 - 20);
        expect(m.data[y * 40 + x]).toBe(d <= 10 ? 255 : 0);
      }
  });

  test('FR-EDT-15 the eraser is the same stamp with destination-out', () => {
    const m = emptyMask({ x: 0, y: 0, w: 40, h: 40 });
    m.data.fill(255);
    stamp(m, 20, 20, 10, 0, 'erase');
    // Pixel (20, 20) has its centre 0.71 px from the dab centre.
    expect(m.data[20 * 40 + 20]).toBe(Math.round(255 * (1 - stampAlpha(Math.SQRT1_2, 10, 0))));
    expect(m.data[0]).toBe(255);
    // 50 % alpha erase of 255 → 128 (rounded).
    const n = emptyMask({ x: 0, y: 0, w: 1, h: 1 });
    n.data[0] = 255;
    stamp(n, 0.5 + 7.5, 0.5, 10, 0.5, 'erase');
    expect(n.data[0]).toBe(128);
  });

  test('FR-EDT-15 dabs are spaced max(1, 0.25 r) along the path, carrying the remainder', () => {
    const a = dabsAlong({ x: 0, y: 0 }, { x: 25, y: 0 }, 10, 0);
    expect(a.dabs.map((d) => d.x)).toEqual([10, 20]);
    expect(a.carry).toBeCloseTo(5);
    const b = dabsAlong({ x: 25, y: 0 }, { x: 40, y: 0 }, 10, a.carry);
    expect(b.dabs.map((d) => d.x)).toEqual([30, 40]);
  });

  test('FR-EDT-19 FR-EDT-20 remapping keeps the overlap and zeroes the rest', () => {
    const m = emptyMask({ x: 100, y: 100, w: 4, h: 4 });
    m.data.fill(200);
    const r = remap(m, { x: 102, y: 98, w: 4, h: 4 });
    // Overlap: x 102..104, y 100..102 → new-rect cols 0..1, rows 2..3.
    expect(Array.from(r.data)).toEqual([0, 0, 0, 0, 0, 0, 0, 0, 200, 200, 0, 0, 200, 200, 0, 0]);
    expect(remap(m, { x: 0, y: 0, w: 2, h: 2 }).data.every((v) => v === 0)).toBe(true);
  });
});

describe('region geometry', () => {
  test('FR-EDT-27 drag draws, moves and resizes integer rects clamped to the image', () => {
    expect(rectFromPoints({ x: 10.6, y: 20.2 }, { x: 5.1, y: 40.9 }, image)).toEqual({
      x: 5,
      y: 20,
      w: 6,
      h: 21,
    });
    expect(rectFromPoints({ x: -50, y: -50 }, { x: 3000, y: 10 }, image)).toEqual({
      x: 0,
      y: 0,
      w: 2000,
      h: 10,
    });
    const r = { x: 100, y: 100, w: 200, h: 100 };
    expect(moveRect(r, 25.4, -10.6, image)).toEqual({ x: 125, y: 89, w: 200, h: 100 });
    expect(moveRect(r, 5000, 5000, image)).toEqual({ x: 1800, y: 1400, w: 200, h: 100 });
    expect(resizeRect(r, 'se', { x: 350, y: 260 }, image)).toEqual({
      x: 100,
      y: 100,
      w: 250,
      h: 160,
    });
    expect(resizeRect(r, 'w', { x: 50, y: 0 }, image)).toEqual({ x: 50, y: 100, w: 250, h: 100 });
    expect(resizeRect(r, 'n', { x: 0, y: 150 }, image)).toEqual({ x: 100, y: 150, w: 200, h: 50 });
    // Dragging a handle past the opposite edge flips, still a valid rect.
    expect(resizeRect(r, 'e', { x: 60, y: 0 }, image)).toEqual({ x: 60, y: 100, w: 40, h: 100 });
    expect(
      applyDrag({ kind: 'move', start: { x: 0, y: 0 }, rect: r }, { x: 1, y: 2 }, image),
    ).toEqual({
      x: 101,
      y: 102,
      w: 200,
      h: 100,
    });
  });

  test('FR-EDT-27 the mocked fit validates like FR-RGN-01/02 (margin 128, clamped)', () => {
    expect(mockRegionFit({ x: 50, y: 60, w: 100, h: 100 }, image)).toEqual({
      inner: { x: 50, y: 60, w: 100, h: 100 },
      outer: { x: 0, y: 0, w: 278, h: 288 },
    });
    expect(() => mockRegionFit({ x: 0, y: 0, w: 10, h: 100 }, image)).toThrow(RegionError);
  });
});

describe('undo stack', () => {
  test('FR-EDT-21 a new action clears redo; FR-EDT-22 the cap drops the oldest, keeps the newest', async () => {
    const u = new UndoStack(100);
    const log: string[] = [];
    const e = (n: string, bytes: number) => ({
      label: n,
      bytes,
      undo: () => void log.push(`-${n}`),
      redo: () => void log.push(`+${n}`),
    });
    u.push(e('a', 40));
    u.push(e('b', 40));
    await u.undo();
    expect(u.snapshot()).toMatchObject({ depth: 1, redo: 1 });
    u.push(e('c', 40));
    expect(u.snapshot()).toMatchObject({ depth: 2, redo: 0, label: 'c' });
    u.push(e('d', 40)); // 120 > 100 → drop a
    expect(u.snapshot()).toMatchObject({ depth: 2, bytes: 80 });
    u.push(e('huge', 500)); // alone over budget: still kept
    expect(u.snapshot()).toMatchObject({ depth: 1, label: 'huge' });
    expect(log).toEqual(['-b']);
  });

  test('FR-EDT-23 a failing entry leaves the position unchanged and shows its code', async () => {
    const u = new UndoStack();
    u.push({
      label: 'accept',
      bytes: 0,
      undo: () => Promise.reject({ code: 'locked', message: 'locked' }),
      redo: () => undefined,
    });
    expect(await u.undo()).toBe(false);
    expect(u.snapshot()).toMatchObject({ depth: 1, redo: 0, error: 'locked', busy: false });
  });
});

describe('mask controller', () => {
  const view: MaskView = { reset: () => undefined, update: () => undefined };
  const region = (x: number, y: number, w: number, h: number): RegionState => ({
    inner: { x, y, w, h },
    outer: { x, y, w, h },
  });
  const setup = () => {
    const undo = new UndoStack();
    let r: RegionState | null = null;
    const c = new MaskController({ undo, view, setRegion: (x) => (r = x) });
    return { undo, c, region: () => r };
  };

  test('FR-EDT-17 without a region painting is refused', () => {
    const { c } = setup();
    expect(c.beginStroke({ x: 1, y: 1 }, 'paint', 10, 1)).toBe(false);
  });

  test('FR-EDT-14 FR-EDT-21 one stroke is one entry; undo then redo is byte-identical', async () => {
    const { c, undo } = setup();
    c.changeRegion(region(100, 100, 64, 64));
    const empty = maskHash(c.buffer!.data);
    c.beginStroke({ x: 110, y: 110 }, 'paint', 12, 0.3);
    c.moveStroke({ x: 150, y: 130 });
    c.moveStroke({ x: 300, y: 300 }); // outside the region: clipped
    c.endStroke();
    const painted = maskHash(c.buffer!.data);
    expect(painted).not.toBe(empty);
    expect(undo.snapshot()).toMatchObject({ depth: 2, label: 'brush stroke' });
    await undo.undo();
    expect(maskHash(c.buffer!.data)).toBe(empty);
    await undo.redo();
    expect(maskHash(c.buffer!.data)).toBe(painted);
  });

  test('FR-EDT-16 fill and clear set every pixel and are one undo step each', async () => {
    const { c, undo } = setup();
    c.changeRegion(region(0, 0, 8, 8));
    c.fillAll(255);
    expect(c.buffer!.data.every((v) => v === 255)).toBe(true);
    c.fillAll(0);
    expect(c.buffer!.data.every((v) => v === 0)).toBe(true);
    await undo.undo();
    expect(c.buffer!.data.every((v) => v === 255)).toBe(true);
    expect(undo.snapshot().depth).toBe(2);
  });

  test('FR-EDT-20 a region change keeps the overlap; removing it discards the mask; each undoable', async () => {
    const { c, undo, region: current } = setup();
    c.changeRegion(region(0, 0, 10, 10));
    c.fillAll(255);
    c.changeRegion(region(5, 5, 10, 10));
    const m = c.buffer!;
    expect(m.data[0]).toBe(255); // (5,5) was inside
    expect(m.data[9 * 10 + 9]).toBe(0); // (14,14) was outside
    c.changeRegion(null);
    expect(c.buffer).toBeNull();
    expect(current()).toBeNull();
    await undo.undo();
    expect(current()?.inner).toEqual({ x: 5, y: 5, w: 10, h: 10 });
    await undo.undo();
    expect(current()?.inner).toEqual({ x: 0, y: 0, w: 10, h: 10 });
    expect(c.buffer!.data.every((v) => v === 255)).toBe(true);
  });

  test('FR-EDT-19 loading a draft mask on a different rect keeps the overlap, rest 0', () => {
    const { c, undo } = setup();
    const saved = emptyMask({ x: 0, y: 0, w: 4, h: 4 });
    saved.data.fill(9);
    c.load(region(2, 0, 4, 4), saved);
    expect(Array.from(c.buffer!.data.subarray(0, 4))).toEqual([9, 9, 0, 0]);
    expect(undo.snapshot().depth).toBe(0); // loading isn't an undo step
  });
});
