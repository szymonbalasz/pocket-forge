import { beforeEach, describe, expect, test, vi } from 'vitest';

import type { ExportPlan, ExportSettingsInput } from '../../api';

const h = vi.hoisted(() => {
  const handlers = new Map<string, (p: unknown) => void>();
  return {
    handlers,
    emit: (event: string, payload: unknown) => handlers.get(event)?.(payload),
    exp: {
      getSettings: vi.fn(),
      setSettings: vi.fn(),
      plan: vi.fn(),
      start: vi.fn(),
      cancel: vi.fn(),
      prefetchOriginal: vi.fn(),
    },
    prj: { close: vi.fn(), open: vi.fn(), get: vi.fn() },
  };
});

vi.mock('../../api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../api')>();
  return {
    ...real,
    api: { export: h.exp, project: h.prj },
    onEvent: async (name: string, fn: (p: unknown) => void) => {
      h.handlers.set(name, fn);
      return () => h.handlers.delete(name);
    },
  };
});

const { ApiError } = await import('../../api');
const { exportStore } = await import('./store');
const { projectStore } = await import('../../store/project');

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

const coreError = (code: string, extra: Record<string, unknown> = {}) =>
  new ApiError({ code, message: `core: ${code}`, ...extra });

async function bound(): Promise<void> {
  await exportStore.listen();
  await exportStore.bind('p1');
}

beforeEach(() => {
  exportStore.reset();
  for (const f of Object.values(h.exp)) f.mockReset();
  h.exp.getSettings.mockResolvedValue({
    grid: { mode: 'cells', cols: 30, rows: 40 },
    target_px_per_square: null,
    format: 'png',
    source_px_per_square: 140,
  });
  h.exp.setSettings.mockImplementation(async (s: ExportSettingsInput) => ({
    ...s,
    source_px_per_square: 140,
  }));
  h.exp.plan.mockResolvedValue(plan());
  h.exp.prefetchOriginal.mockResolvedValue(null);
  h.exp.start.mockResolvedValue({ export_id: 'e1' });
  h.exp.cancel.mockResolvedValue({});
});

describe('export store', () => {
  test('FR-EXP-25 open loads the saved settings and plans them without a commit', async () => {
    await bound();
    expect(exportStore.get().form).toMatchObject({ mode: 'cells', cols: '30', rows: '40' });
    expect(h.exp.setSettings).toHaveBeenCalledWith({
      grid: { mode: 'cells', source_px_per_square: null, cols: 30, rows: 40 },
      target_px_per_square: null,
      format: 'png',
    });
    expect(exportStore.get().plan).toEqual(plan());
    // Typing is session state only: no start (the only EXP command that commits).
    await exportStore.edit({ target: '70' });
    await exportStore.edit({ target: '7' });
    expect(h.exp.start).not.toHaveBeenCalled();
  });

  test('FR-EXP-26 every edit sends the settings to the core (which emits settings-changed)', async () => {
    await bound();
    h.exp.setSettings.mockClear();
    await exportStore.edit({ target: '70' });
    expect(h.exp.setSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ target_px_per_square: 70 }),
    );
  });

  test('FR-EXP-08 invalid_settings from the core lands on its field and blocks the plan', async () => {
    await bound();
    h.exp.setSettings.mockRejectedValueOnce(
      coreError('invalid_settings', { field: 'cols', detail: { limit: { max: 1000 } } }),
    );
    await exportStore.edit({ cols: '1001' });
    expect(exportStore.get().fieldError).toEqual({ field: 'cols', message: 'At most 1000' });
    expect(exportStore.get().plan).toBeNull();
    await exportStore.edit({ cols: '30' });
    expect(exportStore.get().fieldError).toBeNull();
  });

  test('FR-EXP-24 dest_unavailable from the plan is shown in place of the plan', async () => {
    h.exp.plan.mockRejectedValueOnce(coreError('dest_unavailable'));
    await bound();
    expect(exportStore.get().planError).toMatch(/isn't available/);
  });

  test('FR-EXP-28 expanding prefetches the original; original-ready enables Start', async () => {
    await bound();
    expect(h.exp.prefetchOriginal).not.toHaveBeenCalled();
    exportStore.setExpanded(true);
    expect(h.exp.prefetchOriginal).toHaveBeenCalledTimes(1);
    expect(exportStore.get().original.kind).toBe('loading');
    h.emit('project:original-ready', null);
    expect(exportStore.get().original.kind).toBe('ready');
    exportStore.setExpanded(false);
    exportStore.setExpanded(true);
    expect(h.exp.prefetchOriginal).toHaveBeenCalledTimes(1);
  });

  test('FR-EXP-28 a prefetch that says ready, or fails, is honoured', async () => {
    h.exp.prefetchOriginal.mockResolvedValueOnce({ ready: true });
    await bound();
    exportStore.focus();
    await vi.waitFor(() => expect(exportStore.get().original.kind).toBe('ready'));
    exportStore.reset();
    h.exp.prefetchOriginal.mockRejectedValueOnce(coreError('not_available_offline'));
    await bound();
    exportStore.setExpanded(true);
    await vi.waitFor(() => expect(exportStore.get().original.kind).toBe('failed'));
  });

  test('FR-EXP-15 Start replaces only when the plan says the file exists', async () => {
    await bound();
    await exportStore.start();
    expect(h.exp.start).toHaveBeenLastCalledWith(expect.anything(), false);
    h.emit('export:done', doneEvent('e1'));
    h.exp.plan.mockResolvedValue(plan({ exists: { bytes: 1000, mtime: '2026-11-08T00:00:00Z' } }));
    await exportStore.refreshPlan();
    await exportStore.start();
    expect(h.exp.start).toHaveBeenLastCalledWith(expect.anything(), true);
  });

  test('FR-EXP-15 an exists error re-plans so the button turns into Replace', async () => {
    await bound();
    h.exp.start.mockRejectedValueOnce(coreError('exists', { detail: { bytes: 5, mtime: 'x' } }));
    h.exp.plan.mockResolvedValue(plan({ exists: { bytes: 5, mtime: '2026-11-08T00:00:00Z' } }));
    await exportStore.start();
    expect(exportStore.get().plan?.exists).not.toBeNull();
    expect(exportStore.get().run).toBeNull();
  });

  test('FR-EXP-19 progress events move the bar; FR-EXP-27 done shows the toast and re-plans', async () => {
    await bound();
    await exportStore.start();
    h.emit('export:progress', { export_id: 'e1', phase: 'encode', fraction: 0.4 });
    expect(exportStore.get().run).toMatchObject({ exportId: 'e1', phase: 'encode', fraction: 0.4 });
    // Another export's events are ignored.
    h.emit('export:progress', { export_id: 'other', phase: 'write', fraction: 0.9 });
    expect(exportStore.get().run?.fraction).toBe(0.4);
    h.exp.plan.mockClear();
    h.emit('export:done', doneEvent('e1', { record_saved: false }));
    expect(exportStore.get().run).toBeNull();
    expect(exportStore.get().done).toMatchObject({
      path: 'C:/lib/Maps/Crypt/exports/Crypt.png',
      record_saved: false,
    });
    expect(h.exp.plan).toHaveBeenCalled();
    exportStore.dismissDone();
    expect(exportStore.get().done).toBeNull();
  });

  test('FR-EXP-19 a rescale shows one forward-only overall fraction across phases', async () => {
    h.exp.plan.mockResolvedValue(plan({ scale: 0.5, width: 2100, height: 2800 }));
    await bound();
    await exportStore.start();
    const seen: number[] = [];
    for (const [phase, fraction] of [
      ['compose', 0.94],
      ['resize', 0],
      ['resize', 1],
      ['encode', 0.06],
      ['encode', 0.94],
      ['write', 0.5],
      ['write', 0.5],
    ] as const) {
      h.emit('export:progress', { export_id: 'e1', phase, fraction });
      seen.push(exportStore.get().run!.overall);
    }
    expect(seen).toEqual([...seen].sort((a, b) => a - b));
    expect(seen[0]).toBeGreaterThan(0.3);
    expect(seen.at(-1)).toBeGreaterThan(0.95);
    expect(exportStore.get().run).toMatchObject({ phase: 'write', fraction: 0.5 });
  });

  test('FR-EXP-19 events that arrive before export_start returns still apply', async () => {
    await bound();
    let resolve!: (v: { export_id: string }) => void;
    h.exp.start.mockReturnValueOnce(new Promise((r) => (resolve = r)));
    const p = exportStore.start();
    h.emit('export:progress', { export_id: 'e9', phase: 'compose', fraction: 0.2 });
    expect(exportStore.get().run?.fraction).toBe(0.2);
    resolve({ export_id: 'e9' });
    await p;
    expect(exportStore.get().run?.exportId).toBe('e9');
  });

  test('FR-EXP-20 Cancel calls export_cancel once; canceled clears the run', async () => {
    await bound();
    await exportStore.start();
    await exportStore.cancel();
    await exportStore.cancel();
    expect(h.exp.cancel).toHaveBeenCalledTimes(1);
    expect(h.exp.cancel).toHaveBeenCalledWith('e1');
    expect(exportStore.get().run?.canceling).toBe(true);
    h.emit('export:canceled', { export_id: 'e1' });
    expect(exportStore.get().run).toBeNull();
    expect(exportStore.get().error).toBeNull();
  });

  test('OQ-EXP-09 closing during an export asks first; confirm cancels, then closes', async () => {
    await bound();
    await exportStore.start();
    h.prj.close.mockReset();
    // `close` resolves once the guard's answer is in (R55).
    const kept = projectStore.close();
    expect(exportStore.get().closeAsk).toBe(true);
    exportStore.keepExporting();
    await kept;
    expect(exportStore.get().closeAsk).toBe(false);
    expect(h.exp.cancel).not.toHaveBeenCalled();
    expect(h.prj.close).not.toHaveBeenCalled();
    // PRJ's "Close anyway" (force past `unsaved`) is guarded too, and keeps its force.
    const closing = projectStore.close(true);
    await exportStore.confirmClose();
    await closing;
    expect(h.exp.cancel).toHaveBeenCalledWith('e1');
    expect(h.prj.close).toHaveBeenCalledTimes(1);
    expect(h.prj.close).toHaveBeenLastCalledWith(true);
    h.emit('export:canceled', { export_id: 'e1' });
    // Nothing running: the guard lets a close straight through.
    await projectStore.close();
    expect(exportStore.get().closeAsk).toBe(false);
    expect(h.prj.close).toHaveBeenCalledTimes(2);
    expect(h.prj.close).toHaveBeenLastCalledWith(false);
  });

  test('FR-EXP-18 a failed export shows its message and Retry starts again', async () => {
    await bound();
    await exportStore.start();
    h.emit('export:failed', { export_id: 'e1', code: 'locked', detail: null });
    expect(exportStore.get().error).toEqual({
      code: 'locked',
      message: 'The file is in use (VTT or sync?)',
    });
    await exportStore.retry();
    expect(h.exp.start).toHaveBeenCalledTimes(2);
    expect(exportStore.get().error).toBeNull();
  });

  test('FR-EXP-21 FR-EXP-29 start errors (busy, read_only) are shown and nothing runs', async () => {
    await bound();
    h.exp.start.mockRejectedValueOnce(coreError('busy'));
    await exportStore.start();
    expect(exportStore.get().error?.code).toBe('busy');
    expect(exportStore.get().run).toBeNull();
    h.exp.start.mockRejectedValueOnce(coreError('read_only'));
    await exportStore.start();
    expect(exportStore.get().error?.message).toBe('The project is read-only');
  });

  test('FR-PRJ-42 R104 original-failed shows the readable error instead of loading', async () => {
    await bound();
    exportStore.setExpanded(true);
    h.emit('project:original-failed', { project_id: 'other', code: 'corrupt' });
    expect(exportStore.get().original.kind).toBe('loading');
    h.emit('project:original-failed', { project_id: 'p1', code: 'corrupt' });
    expect(exportStore.get().original).toEqual({
      kind: 'failed',
      message: "The original can't be decoded (the file may be damaged)",
    });
    h.emit('project:original-failed', { project_id: 'p1', code: 'internal' });
    expect(exportStore.get().original).toEqual({
      kind: 'failed',
      message: "The original can't be read",
    });
  });

  test('FR-EXP-28 original_unavailable from start puts Start back to loading', async () => {
    await bound();
    exportStore.setExpanded(true);
    h.emit('project:original-ready', null);
    h.exp.start.mockRejectedValueOnce(coreError('original_unavailable'));
    await exportStore.start();
    expect(exportStore.get().original.kind).toBe('loading');
  });

  test('a stale response never overwrites a newer one', async () => {
    await bound();
    let slow!: () => void;
    h.exp.setSettings.mockReturnValueOnce(new Promise<void>((r) => (slow = r)));
    const first = exportStore.edit({ target: '70' });
    h.exp.plan.mockResolvedValueOnce(plan({ scale: 0.25, width: 1050, height: 1400 }));
    await exportStore.edit({ target: '35' });
    h.exp.plan.mockResolvedValueOnce(plan({ scale: 0.5, width: 2100, height: 2800 }));
    slow();
    await first;
    expect(exportStore.get().plan?.scale).toBe(0.25);
  });

  test('closing the project empties the panel', async () => {
    await bound();
    await exportStore.bind(null);
    expect(exportStore.get()).toMatchObject({ projectId: null, plan: null, run: null });
  });
});

function doneEvent(id: string, p: Record<string, unknown> = {}) {
  return {
    export_id: id,
    path: 'C:/lib/Maps/Crypt/exports/Crypt.png',
    width: 4200,
    height: 5600,
    bytes: 1234567,
    format: 'png',
    fallback: null,
    record_saved: true,
    ...p,
  };
}

test('FR-PRJ-36 after a resolved conflict the form reloads the kept settings (walk 2 N1)', async () => {
  const snap = { project_id: 'p1', warnings: [], revisions: [], committed_at: null };
  h.prj.open.mockResolvedValue(snap);
  h.prj.get.mockResolvedValue(snap);
  await projectStore.open('<library-root>/Maps/Crypt');
  await bound();
  await exportStore.edit({ format: 'jpeg' });
  expect(exportStore.get().form.format).toBe('jpeg');

  // Keep other: the core now reports the kept version's settings.
  h.exp.getSettings.mockResolvedValue({
    grid: { mode: 'cells', cols: 10, rows: 12 },
    target_px_per_square: null,
    format: 'webp',
    source_px_per_square: 420,
  });
  await projectStore.resolved('other');
  await vi.waitFor(() => expect(exportStore.get().form.format).toBe('webp'));
  expect(h.exp.setSettings.mock.calls.at(-1)?.[0]).toMatchObject({ format: 'webp' });
  projectStore.reset();
});
