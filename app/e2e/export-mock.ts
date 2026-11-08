// EXP e2e helpers on top of mock-core: every `export_*` command and event exactly per EXP §9
// (names, payloads, errors), until EXP slice 3 lands the real ones. The size maths runs in Node
// through `src/features/export/sizing.ts` (the mirror of `pf-imaging` sizing.rs), so the mocked
// plan is the core's plan. State lives in the page (window.__pfExp) like the other feature mocks.
import { expect, type Page } from '@playwright/test';

import type { CoreErrorShape, ExportPlan, ExportSettingsInput } from '../src/api/types';
import {
  exportPath,
  formatUnavailable,
  InvalidSetting,
  planSize,
  WARNING_TEXT,
} from '../src/features/export/sizing';
import { openMap } from './edt-mock';
import { defaultSeed } from './mock-core';

export interface ExportSeed {
  /** Project (folder) name: the export file name stem (FR-EXP-14). */
  name: string;
  image: { w: number; h: number };
  readOnly: 'newer_version' | 'original_changed' | 'original_missing' | null;
  /** Settings saved in the manifest `export` record (null = never set). */
  saved: ExportSettingsInput | null;
  /** Files already in `exports/`, project-relative, with their age. */
  existing: { path: string; bytes: number; ageMs: number }[];
  /** Delay after `project_prefetch_original` before `project:original-ready`; null = never. */
  originalReadyMs: number | null;
  /** Progress steps and their interval for one export. */
  steps: number;
  stepMs: number;
  /** `export:failed` code for the next run(s), at half way (`locked`, `disk_full`, …). */
  failRun: string | null;
  /** How many runs fail with `failRun` before the rest succeed. */
  failRunTimes: number;
  /** WebP partition overflow → lossless fallback (FR-EXP-12). */
  webpFallback: boolean;
  /** `export:done.record_saved` (§11). */
  recordSaved: boolean;
}

export const CRYPT = { w: 4200, h: 5600 };
export const PROJECT_DIR = 'C:/pf-e2e/library/Maps/Crypt';

export const defaultExportSeed: ExportSeed = {
  name: 'Crypt',
  image: CRYPT,
  readOnly: null,
  saved: null,
  existing: [],
  originalReadyMs: 50,
  steps: 6,
  stepMs: 80,
  failRun: null,
  failRunTimes: 1,
  webpFallback: false,
  recordSaved: true,
};

interface EvalInput {
  settings: ExportSettingsInput | null;
  w: number;
  h: number;
  name: string;
  existing: Record<string, { bytes: number; mtime: string }>;
}

type EvalResult =
  { ok: true; plan: ExportPlan; source: number | null } | { ok: false; error: CoreErrorShape };

/** The core's `export_plan` maths (FR-EXP-06..08, 11, 13, 14, 24), in Node. */
function evalPlan({ settings, w, h, name, existing }: EvalInput): EvalResult {
  const s: ExportSettingsInput = settings ?? {
    grid: { mode: 'px', source_px_per_square: null },
    target_px_per_square: null,
    format: 'png',
  };
  try {
    const size = planSize(w, h, s.grid, s.target_px_per_square);
    const path = exportPath(name, size.target, s.format);
    const mib = 1024 * 1024;
    const full = (w * h * 3) / mib;
    const out = (size.width * size.height * 3) / mib;
    const streams = size.scale === 1 && s.format !== 'webp';
    const est = Math.ceil(
      streams ? (w * 512 * 3 * 2) / mib : full + out * (s.format === 'webp' ? 2 : 1),
    );
    const warnings: { code: string; message: string }[] = size.warnings.map((code) => ({
      code,
      message: WARNING_TEXT[code],
    }));
    if (est > 2048) warnings.push({ code: 'memory', message: `Needs about ${est} MiB of memory` });
    return {
      ok: true,
      source: size.source,
      plan: {
        width: size.width,
        height: size.height,
        scale: size.scale,
        formats: (['png', 'jpeg', 'webp'] as const).map((format) => {
          const reason = formatUnavailable(format, size.width, size.height);
          return { format, available: reason === null, reason };
        }),
        path,
        exists: existing[path] ?? null,
        est_peak_mib: est,
        warnings,
      },
    };
  } catch (e) {
    if (e instanceof InvalidSetting) return { ok: false, error: e.toCoreError() };
    throw e;
  }
}

/** Installs the EXP mock, then opens the seeded project (EDT's `openMap`). */
export async function openExport(page: Page, seed: Partial<ExportSeed> = {}): Promise<void> {
  const s = { ...defaultExportSeed, ...seed };
  await page.exposeFunction('__pfExpEval', evalPlan);
  await page.addInitScript(
    ({ seed, projectDir }) => {
      type Rec = Record<string, unknown>;
      const w = window as unknown as Rec;
      const h = (w.__pfMockHandlers ??= {}) as Record<string, (input: Rec) => unknown>;
      w.__pfMockSnapshot = {
        name: seed.name,
        project_dir: projectDir,
        read_only: seed.readOnly,
      };
      const evaluate = w.__pfExpEval as (i: unknown) => Promise<
        | {
            ok: true;
            plan: Rec & { path: string; exists: unknown; formats: Rec[] };
            source: number | null;
          }
        | { ok: false; error: Rec }
      >;
      const emit = (event: string, payload: unknown) =>
        (w.__pfMock as { emit: (e: string, p: unknown) => void }).emit(event, payload);
      const now = Date.now();
      const st = {
        settings: seed.saved as Rec | null,
        existing: Object.fromEntries(
          seed.existing.map((f) => [
            f.path,
            { bytes: f.bytes, mtime: new Date(now - f.ageMs).toISOString() },
          ]),
        ) as Record<string, { bytes: number; mtime: string }>,
        originalReady: false,
        running: null as null | { id: string; timer: number },
        nextId: 1,
        failsLeft: seed.failRunTimes,
        lastEmitted: undefined as undefined | string,
        starts: [] as Rec[],
      };
      w.__pfExp = st;
      const fail = (code: string, message: string, extra: Rec = {}) =>
        Promise.reject({ code, message, ...extra });
      const run = (settings: Rec | null) =>
        evaluate({
          settings,
          w: seed.image.w,
          h: seed.image.h,
          name: seed.name,
          existing: st.existing,
        });
      const withSource = (settings: Rec | null, source: number | null) => ({
        ...(settings ?? {}),
        source_px_per_square: source,
        target_px_per_square: (settings?.target_px_per_square as number | null | undefined) ?? null,
      });

      h.export_get_settings = async () => {
        const r = await run(st.settings);
        return withSource(st.settings, r.ok ? r.source : null);
      };
      h.export_set_settings = async (input) => {
        const settings = input.settings as Rec;
        const r = await run(settings);
        if (!r.ok) return Promise.reject(r.error);
        st.settings = settings;
        const out = withSource(settings, r.source);
        const ev = { source: r.source, target: out.target_px_per_square };
        // `export:settings-changed` on every change of the two values (FR-EXP-26).
        if (JSON.stringify(ev) !== st.lastEmitted) {
          st.lastEmitted = JSON.stringify(ev);
          emit('export:settings-changed', ev);
        }
        return out;
      };
      h.export_plan = async (input) => {
        const r = await run(input.settings as Rec);
        return r.ok ? r.plan : Promise.reject(r.error);
      };
      h.project_prefetch_original = () => {
        if (seed.originalReadyMs !== null)
          setTimeout(() => {
            st.originalReady = true;
            emit('project:original-ready', null);
          }, seed.originalReadyMs);
        return null;
      };
      h.export_start = async (input) => {
        st.starts.push(input);
        if (seed.readOnly) return fail('read_only', 'the project is read-only');
        if (st.running) return fail('busy', 'an export is already running');
        if (!st.originalReady)
          return fail('original_unavailable', 'the original is not decoded yet');
        const settings = input.settings as Rec;
        const r = await run(settings);
        if (!r.ok) return Promise.reject(r.error);
        const plan = r.plan;
        const fmt = plan.formats.find((f) => f.format === settings.format)!;
        if (!fmt.available)
          return fail('too_large', String(fmt.reason), {
            detail: { format: settings.format, limit: 16383 },
          });
        if (plan.exists && input.replace !== true)
          return fail('exists', 'the file exists', { detail: plan.exists });
        st.settings = settings;
        const id = `01JEXPORT${String(st.nextId++).padStart(17, '0')}`;
        const failCode = st.failsLeft > 0 ? seed.failRun : null;
        if (failCode) st.failsLeft--;
        const rescales = plan.scale !== 1;
        let i = 0;
        const tick = () => {
          i++;
          const fraction = i / seed.steps;
          if (failCode && fraction >= 0.5) {
            st.running = null;
            emit('export:failed', { export_id: id, code: failCode, detail: null });
            return;
          }
          if (i >= seed.steps) {
            st.running = null;
            const bytes = Math.round((Number(plan.width) * Number(plan.height) * 3) / 4);
            st.existing[plan.path] = { bytes, mtime: new Date().toISOString() };
            emit('export:done', {
              export_id: id,
              path: `${projectDir}/${plan.path}`,
              width: plan.width,
              height: plan.height,
              bytes,
              format: settings.format,
              fallback: settings.format === 'webp' && seed.webpFallback ? 'lossless' : null,
              record_saved: seed.recordSaved,
            });
            return;
          }
          const phase =
            fraction < 0.4
              ? 'compose'
              : rescales && fraction < 0.6
                ? 'resize'
                : fraction < 0.9
                  ? 'encode'
                  : 'write';
          emit('export:progress', { export_id: id, phase, fraction });
          st.running = { id, timer: window.setTimeout(tick, seed.stepMs) };
        };
        st.running = { id, timer: window.setTimeout(tick, seed.stepMs) };
        return { export_id: id };
      };
      h.export_cancel = (input) => {
        if (!st.running || st.running.id !== input.export_id)
          return fail('unknown_id', 'no such export (already finished?)');
        clearTimeout(st.running.timer);
        st.running = null;
        setTimeout(() => emit('export:canceled', { export_id: input.export_id }), 20);
        return {};
      };
    },
    { seed: s, projectDir: PROJECT_DIR },
  );
  // Copy path: record what the UI writes (the sandbox has no clipboard read).
  await page.addInitScript(() => {
    const w = window as unknown as { __pfClipboard: string[] };
    w.__pfClipboard = [];
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (t: string) => void w.__pfClipboard.push(t) },
    });
  });
  await openMap(page, { seed: { ...defaultSeed, image: s.image } });
}

/** The mock's state (files in `exports/`, `export_start` inputs). */
export function exportState(page: Page): Promise<{
  existing: Record<string, { bytes: number; mtime: string }>;
  starts: { settings: ExportSettingsInput; replace: boolean }[];
  running: unknown;
}> {
  return page.evaluate(() => {
    const st = (window as unknown as { __pfExp: Record<string, unknown> }).__pfExp;
    return { existing: st.existing, starts: st.starts, running: st.running } as never;
  });
}

/** Expands the Export section with Ctrl+E and waits for the original. */
export async function expandExport(page: Page, waitReady = true): Promise<void> {
  await page.keyboard.press('Control+e');
  await expect(page.getByTestId('exp-panel-grid-mode')).toBeVisible();
  if (waitReady)
    await expect(page.getByTestId('exp-panel')).toHaveAttribute('data-state-panel', 'ready');
}
