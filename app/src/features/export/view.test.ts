import { describe, expect, test } from 'vitest';

import type { ExportPlan } from '../../api';
import {
  ago,
  emptyForm,
  errorText,
  formatBytes,
  formHasSource,
  formOf,
  invalidSettingsError,
  overallFraction,
  phaseWeights,
  planLine,
  replaceNote,
  settingsOf,
  startState,
} from './view';

const plan = (p: Partial<ExportPlan> = {}): ExportPlan => ({
  width: 4200,
  height: 5600,
  scale: 1,
  formats: [
    { format: 'png', available: true, reason: null },
    { format: 'jpeg', available: true, reason: null },
    { format: 'webp', available: true, reason: null },
  ],
  path: 'exports/Crypt.png',
  exists: null,
  est_peak_mib: 60,
  warnings: [],
  ...p,
});

const ready = {
  hasProject: true,
  readOnly: null,
  original: { kind: 'ready' } as const,
  plan: plan(),
  format: 'png' as const,
  running: false,
  blocked: false,
};

describe('export panel view', () => {
  test('FR-EXP-06 cells mode sends cols and rows; px mode sends the source', () => {
    const cells = settingsOf({ ...emptyForm, mode: 'cells', cols: '30', rows: '40' });
    expect(cells).toEqual({
      settings: {
        grid: { mode: 'cells', source_px_per_square: null, cols: 30, rows: 40 },
        target_px_per_square: null,
        format: 'png',
      },
    });
    const px = settingsOf({ ...emptyForm, source: '140.5', target: '70' });
    expect(px).toMatchObject({
      settings: { grid: { mode: 'px', source_px_per_square: 140.5 }, target_px_per_square: 70 },
    });
  });

  test('FR-EXP-06 a malformed field blocks only in its own grid mode', () => {
    expect(settingsOf({ ...emptyForm, mode: 'cells', cols: '30.5' })).toEqual({
      error: { field: 'cols', message: 'Whole number' },
    });
    expect(settingsOf({ ...emptyForm, mode: 'px', source: 'abc' })).toEqual({
      error: { field: 'source', message: 'Enter a number' },
    });
    // The px field is ignored in cells mode.
    expect(settingsOf({ ...emptyForm, mode: 'cells', source: 'abc', cols: '30' })).toHaveProperty(
      'settings',
    );
  });

  test('FR-EXP-07 the target is disabled and sent as null until the grid gives a source', () => {
    const f = { ...emptyForm, mode: 'cells' as const, target: '70' };
    expect(formHasSource(f)).toBe(false);
    expect(settingsOf(f)).toMatchObject({ settings: { target_px_per_square: null } });
    expect(formHasSource({ ...f, cols: '30' })).toBe(true);
    expect(settingsOf({ ...f, cols: '30' })).toMatchObject({
      settings: { target_px_per_square: 70 },
    });
  });

  test('FR-EXP-25 the form restores from export_get_settings', () => {
    const f = formOf({
      grid: { mode: 'cells', cols: 30, rows: 40 },
      target_px_per_square: 70,
      format: 'webp',
      source_px_per_square: 140,
    });
    expect(f).toEqual({
      mode: 'cells',
      source: '',
      cols: '30',
      rows: '40',
      target: '70',
      format: 'webp',
    });
    // Older or partial results fall back to the defaults.
    expect(formOf({ source_px_per_square: null, target_px_per_square: null })).toEqual(emptyForm);
  });

  test('FR-EXP-08 invalid_settings limits in words, on their field', () => {
    expect(invalidSettingsError('cols', { max: 1000 })).toEqual({
      field: 'cols',
      message: 'At most 1000',
    });
    expect(invalidSettingsError('source_px_per_square', { min: 8 })).toEqual({
      field: 'source',
      message: 'At least 8',
    });
    expect(invalidSettingsError('scale', { max: 4 })).toEqual({
      field: 'target',
      message: 'At most 4× upscale',
    });
    expect(invalidSettingsError('width', { max: 16384 }).message).toBe(
      'Output would be over 16384 px',
    );
    expect(
      invalidSettingsError('target_px_per_square', { requires: 'source_px_per_square' }).message,
    ).toBe('Set the grid first');
  });

  test('FR-EXP-08 FR-EXP-14 the plan line shows sizes, scale and the file', () => {
    expect(
      planLine(
        4200,
        5600,
        plan({ width: 2100, height: 2800, scale: 0.5, path: 'exports/Crypt_70px.png' }),
      ),
    ).toBe('4200×5600 → 2100×2800 · 0.5× · exports/Crypt_70px.png');
    expect(planLine(4200, 5600, plan())).toBe('4200×5600 · Full resolution · exports/Crypt.png');
    expect(planLine(300, 300, plan({ width: 100, height: 100, scale: 1 / 3 }))).toContain('0.333×');
  });

  test('FR-EXP-15 Replace wording when the destination exists', () => {
    const now = Date.parse('2026-11-08T12:00:00Z');
    const p = plan({
      path: 'exports/Crypt_70px.png',
      exists: { bytes: 12.3 * 1024 * 1024, mtime: '2026-11-06T12:00:00Z' },
    });
    expect(replaceNote(p, now)).toBe('Replaces Crypt_70px.png (12.3 MB, 2 days ago)');
    expect(replaceNote(plan(), now)).toBeNull();
    expect(startState({ ...ready, plan: p })).toEqual({
      enabled: true,
      label: 'Replace',
      reason: null,
    });
    expect(startState(ready).label).toBe('Export');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(2048)).toBe('2 KB');
    expect(ago('2026-11-08T11:59:59Z', now)).toBe('now');
    expect(ago('2026-11-07T12:00:00Z', now)).toBe('yesterday');
  });

  test('FR-EXP-11 Start is disabled with the reason when the chosen format is unavailable', () => {
    const p = plan({
      formats: [
        { format: 'png', available: true, reason: null },
        { format: 'jpeg', available: true, reason: null },
        { format: 'webp', available: false, reason: 'WebP max 16383 px' },
      ],
    });
    expect(startState({ ...ready, plan: p, format: 'webp' })).toMatchObject({
      enabled: false,
      reason: 'WebP max 16383 px',
    });
    expect(startState({ ...ready, plan: p, format: 'png' }).enabled).toBe(true);
  });

  test('FR-EXP-28 Start waits for the original', () => {
    expect(startState({ ...ready, original: { kind: 'loading' } })).toMatchObject({
      enabled: false,
      reason: 'Loading original…',
    });
    expect(
      startState({ ...ready, original: { kind: 'failed', message: 'Not available offline' } }),
    ).toMatchObject({ enabled: false, reason: 'Not available offline' });
  });

  test('FR-EXP-29 read-only disables Start with the reason; empty disables everything', () => {
    expect(startState({ ...ready, readOnly: 'original_changed' })).toMatchObject({
      enabled: false,
      reason: 'Read-only: the original image changed',
    });
    expect(startState({ ...ready, hasProject: false, plan: null })).toMatchObject({
      enabled: false,
      reason: 'Open a project to export',
    });
    expect(startState({ ...ready, running: true }).enabled).toBe(false);
    expect(startState({ ...ready, blocked: true }).enabled).toBe(false);
  });

  test('FR-EXP-18 FR-EXP-21 FR-EXP-22 error codes in words', () => {
    expect(errorText('locked')).toBe('The file is in use (VTT or sync?)');
    expect(errorText('busy')).toMatch(/already running/);
    expect(errorText('layer_unreadable')).toMatch(/nothing was exported/);
    expect(errorText('weird', 'core says no')).toBe('core says no');
  });

  test('FR-EXP-19 weighted phases give one 0–100 fraction that never restarts', () => {
    for (const rescale of [false, true])
      for (const format of ['png', 'jpeg', 'webp'] as const) {
        const w = phaseWeights(rescale, format);
        expect(w.reduce((a, [, x]) => a + x, 0)).toBeCloseTo(1);
        let prev = 0;
        for (const [phase] of w)
          for (const f of [0, 0.5, 1]) {
            const o = overallFraction(w, phase, f);
            expect(o).toBeGreaterThanOrEqual(prev);
            prev = o;
          }
        expect(prev).toBeCloseTo(1);
      }
    const w = phaseWeights(true, 'png');
    // Compose done, resize just started: not back to 0.
    expect(overallFraction(w, 'resize', 0)).toBeCloseTo(0.35);
    expect(overallFraction(w, 'encode', 0.5)).toBeCloseTo(0.775);
  });
});
