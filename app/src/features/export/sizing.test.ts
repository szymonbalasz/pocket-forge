// Same vectors as `pf-imaging` sizing.rs, so the mocked plan matches the core's.
import { describe, expect, test } from 'vitest';

import type { ExportGrid, ExportLimit } from '../../api';
import {
  exportFileName,
  exportPath,
  formatUnavailable,
  InvalidSetting,
  planSize,
  sourcePxPerSquare,
} from './sizing';

const cells = (cols: number, rows: number | null = null): ExportGrid => ({
  mode: 'cells',
  cols,
  rows,
});
const px = (v: number): ExportGrid => ({ mode: 'px', source_px_per_square: v });

function invalid(f: () => unknown): { field: string; limit: ExportLimit } {
  try {
    f();
  } catch (e) {
    if (e instanceof InvalidSetting) return { field: e.field, limit: e.limit };
    throw e;
  }
  throw new Error('expected InvalidSetting');
}

describe('export sizing (mirror of sizing.rs)', () => {
  test('FR-EXP-06 cells give source as width over cols', () => {
    expect(sourcePxPerSquare(cells(30, 40), 4200, 5600)).toEqual({ source: 140, warning: null });
  });

  test('FR-EXP-06 a non-square grid warns without blocking', () => {
    expect(planSize(4200, 5600, cells(30, 39), null).warnings).toEqual(['grid_not_square']);
    expect(planSize(4200, 5650, cells(30, 40), null).warnings).toEqual([]);
  });

  test('FR-EXP-06 validates grid ranges', () => {
    const table: [ExportGrid, string, ExportLimit][] = [
      [px(7.9), 'source_px_per_square', { min: 8 }],
      [px(1024.5), 'source_px_per_square', { max: 1024 }],
      [px(NaN), 'source_px_per_square', { min: 8 }],
      [cells(0), 'cols', { min: 1 }],
      [cells(1001), 'cols', { max: 1000 }],
      [cells(10, 0), 'rows', { min: 1 }],
    ];
    for (const [grid, field, limit] of table)
      expect(invalid(() => planSize(4200, 5600, grid, null))).toEqual({ field, limit });
    expect(() => planSize(4200, 5600, px(8), null)).not.toThrow();
    expect(() => planSize(4200, 5600, px(1024), null)).not.toThrow();
  });

  test('FR-EXP-06 an empty grid gives no source yet', () => {
    expect(planSize(4200, 5600, { mode: 'cells', cols: null }, null).source).toBeNull();
    expect(planSize(4200, 5600, { mode: 'px', source_px_per_square: null }, null).source).toBeNull();
  });

  test('FR-EXP-07 no target is full resolution', () => {
    const s = planSize(4200, 5600, cells(30), null);
    expect([s.scale, s.width, s.height]).toEqual([1, 4200, 5600]);
    expect(planSize(4200, 5600, null, null)).toMatchObject({ scale: 1, source: null });
  });

  test('FR-EXP-07 target needs a source and its range', () => {
    expect(invalid(() => planSize(4200, 5600, null, 70))).toEqual({
      field: 'target_px_per_square',
      limit: { requires: 'source_px_per_square' },
    });
    for (const [t, limit] of [
      [7, { min: 8 }],
      [1025, { max: 1024 }],
    ] as const)
      expect(invalid(() => planSize(4200, 5600, cells(30), t))).toEqual({
        field: 'target_px_per_square',
        limit,
      });
  });

  test('FR-EXP-08 output size is the rounded side times scale', () => {
    const s = planSize(4200, 5600, cells(30, 40), 70);
    expect([s.scale, s.width, s.height, s.warnings]).toEqual([0.5, 2100, 2800, []]);
    const r = planSize(1001, 999, px(140), 70);
    expect([r.width, r.height]).toEqual([501, 500]);
  });

  test('FR-EXP-08 upscale warns and the limits apply', () => {
    const s = planSize(1000, 800, px(70), 140);
    expect([s.width, s.height, s.warnings]).toEqual([2000, 1600, ['upscaled']]);
    expect(invalid(() => planSize(100, 100, px(8), 40))).toEqual({
      field: 'scale',
      limit: { max: 4 },
    });
    expect(invalid(() => planSize(10000, 100, px(100), 200))).toEqual({
      field: 'width',
      limit: { max: 16384 },
    });
    expect(invalid(() => planSize(1, 4000, px(1000), 8))).toEqual({
      field: 'width',
      limit: { min: 1 },
    });
  });

  test('FR-EXP-11 WebP is unavailable above 16383 px', () => {
    expect(formatUnavailable('webp', 16383, 16383)).toBeNull();
    expect(formatUnavailable('webp', 16384, 10)).toBe('WebP max 16383 px');
    expect(formatUnavailable('png', 16384, 16384)).toBeNull();
    expect(formatUnavailable('jpeg', 16384, 16384)).toBeNull();
  });

  test('FR-EXP-14 file name from the project name and target', () => {
    expect(exportFileName('Crypt', 70, 'png')).toBe('Crypt_70px.png');
    expect(exportFileName('Crypt', null, 'jpeg')).toBe('Crypt.jpg');
    expect(exportPath('Crypt', null, 'webp')).toBe('exports/Crypt.webp');
  });
});
