// The Size block's pure helpers (FR-GEN-07..10).
import { expect, test } from 'vitest';

import type { Capabilities, GenSizeFit } from '../../api';
import {
  clampGrid,
  clampLongSide,
  defaultSize,
  ratioOptions,
  resultLabel,
  sizeFor,
  WH_RATIOS,
} from './size';
import { GEN_ASPECT, GEN_WH } from './testUtils';

test('FR-GEN-07 FR-GEN-10 OQ-GEN-11 defaults per kind; a saved size of the other mode is replaced', () => {
  expect(defaultSize('map')).toEqual({ mode: 'grid', cols: 30, rows: 40, pps: 70 });
  expect(defaultSize('token')).toEqual({ mode: 'ratio', aspect: '1:1', long_side: 1024 });
  expect(defaultSize('item')).toEqual({ mode: 'ratio', aspect: '1:1', long_side: 1024 });
  expect(sizeFor('token', { mode: 'grid', cols: 1, rows: 1, pps: 16 })).toEqual(
    defaultSize('token'),
  );
  const saved = { mode: 'grid', cols: 12, rows: 9, pps: 50 } as const;
  expect(sizeFor('map', saved)).toBe(saved);
});

test('FR-GEN-10 wh models offer the fixed ratio list; aspect models their own ratios, never a keyword', () => {
  expect(ratioOptions(GEN_WH)).toEqual(WH_RATIOS);
  expect(ratioOptions(GEN_ASPECT)).toEqual(['1:1', '3:4', '4:3', '2:3']);
  const withKeywords: Capabilities = {
    ...GEN_ASPECT,
    output_size: { mode: 'aspect', allowed: ['auto', 'source', '16:9', '1.91:1'] },
  };
  expect(ratioOptions(withKeywords)).toEqual(['16:9', '1.91:1']);
});

test('FR-GEN-07 FR-GEN-10 inputs are clamped: cells 1–200, pps 16–512, long side to the model’s step', () => {
  expect(clampGrid({ cols: 0, rows: 500, pps: 3.4 })).toEqual({ cols: 1, rows: 200, pps: 16 });
  expect(clampLongSide(1000, { min: 64, max: 2048, step: 16 })).toBe(992);
  expect(clampLongSide(5000, { min: 64, max: 2048, step: 16 })).toBe(2048);
  expect(clampLongSide(10, { min: 64, max: 2048, step: 16 })).toBe(64);
});

test('FR-GEN-08 FR-GEN-09 the result line: sent w × h with px per square (1 decimal), or the ratio', () => {
  const grid = { mode: 'grid', cols: 30, rows: 40, pps: 51 } as const;
  expect(resultLabel(grid, { mode: 'wh', w: 1520, h: 2032, pps: 51, notices: [] })).toBe(
    '1520 × 2032 · 50.7 × 50.8 px per square',
  );
  expect(resultLabel(grid, { mode: 'aspect', aspect: '2:3', pps: 51, notices: [] })).toBe(
    'ratio 2:3',
  );
  const ratio = { mode: 'ratio', aspect: '1:1', long_side: 1024 } as const;
  expect(resultLabel(ratio, { mode: 'wh', w: 1024, h: 1024, pps: null, notices: [] })).toBe(
    '1024 × 1024',
  );
});

test('FR-GEN-09 on aspect models the px per square shows once a result of that ratio is in', () => {
  const grid = { mode: 'grid', cols: 25, rows: 40, pps: 70 } as const;
  const fit: GenSizeFit = { mode: 'aspect', aspect: '2:3', pps: 70, notices: [] };
  const result = { aspect: '2:3', w: 683, h: 1024 };
  expect(resultLabel(grid, fit, result)).toBe(
    'ratio 2:3 · result 683 × 1024 · 27.3 × 25.6 px per square',
  );
  expect(resultLabel(grid, fit, { ...result, aspect: '3:4' })).toBe('ratio 2:3');
  const ratio = { mode: 'ratio', aspect: '2:3', long_side: 1024 } as const;
  expect(resultLabel(ratio, { ...fit, pps: null }, result)).toBe('ratio 2:3 · result 683 × 1024');
});
