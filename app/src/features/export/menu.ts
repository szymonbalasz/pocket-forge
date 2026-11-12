// Menu exports (#63, EXP §7): Export as and Save as from the context menus, and Open in Explorer.
// No domain logic: the core runs them (`export_start` with `format_override` / `dest`,
// `export_file`, `reveal_in_explorer`); this keeps the toast's state and follows `export:*`.
// A project's menu export counts as its running export for the close, rename and delete
// questions (FR-EXP-33) through `exportStore.setMenuRun`.
import { useSyncExternalStore } from 'react';

import {
  api,
  ApiError,
  onEvent,
  pickSavePath,
  type Events,
  type ExportFormat,
  type ExportSettingsInput,
} from '../../api';
import { projectStore } from '../../store/project';
import { exportStore } from './store';
import { errorText, fileNameOf } from './view';

/** What a menu export does, so Replace can run it again. */
export type MenuJob =
  | { kind: 'canvas'; format: ExportFormat }
  | { kind: 'canvas-save'; dest: string }
  | { kind: 'file'; source: string; dest: 'exports' | string; format: ExportFormat | null };

export interface MenuToast {
  /** Bumped per toast, so a late answer never updates a newer toast. */
  seq: number;
  phase: 'loading' | 'running' | 'exists' | 'done' | 'failed';
  /** "Exporting as JPEG…", "Saving dragon.png…". */
  label: string;
  exportId: string | null;
  fraction: number;
  canceling: boolean;
  exists: { name: string; bytes: number; mtime: string } | null;
  done: Events['export:done'] | null;
  error: string | null;
  /** A second menu export was asked for while this one runs (walk-63 M1): said here, and this
   *  toast keeps its progress and Cancel. */
  notice: string | null;
  job: MenuJob | null;
}

/** walk-63 M1: one menu export at a time in the toast. */
export const BUSY_NOTICE = 'An export is already running. Wait for it, or cancel it first.';

let toast: MenuToast | null = null;
let seq = 0;
const listeners = new Set<() => void>();
let listening: Promise<void> | null = null;
/** Running menu exports: id → whether it's the open project's (FR-EXP-33). */
const runs = new Map<string, { project: boolean }>();
/** Events that came before `export_start` / `export_file` answered with their id. */
const early = new Map<string, { name: string; payload: unknown }[]>();

function set(next: MenuToast | null): void {
  toast = next;
  for (const l of listeners) l();
}

function patch(at: number, p: Partial<MenuToast>): void {
  if (toast && toast.seq === at) set({ ...toast, ...p });
}

const FORMAT_LABEL: Record<ExportFormat, string> = { png: 'PNG', jpeg: 'JPEG', webp: 'WebP' };

const MENU_ERROR: Record<string, string> = {
  dest_unavailable: "The folder isn't available (moved, offline or not writable)",
  invalid_settings: 'Pick a .png, .jpg or .webp file name',
  not_available_offline: 'The file is only in the cloud and you are offline',
  unsupported: "The file isn't an image Pocket Forge can read",
  corrupt: "The image can't be decoded (the file may be damaged)",
  outside_library: 'The file is outside the library',
  not_found: 'The file is gone (moved or deleted?)',
};

function messageOf(e: unknown): { code: string; text: string; detail?: unknown } {
  if (e instanceof ApiError)
    return {
      code: e.code,
      text: MENU_ERROR[e.code] ?? errorText(e.code, e.message),
      detail: e.detail,
    };
  return { code: 'ui', text: String(e) };
}

/** A run of the open project ended (or never started): Start and the close questions follow. */
function finish(exportId: string): void {
  const r = runs.get(exportId);
  runs.delete(exportId);
  if (r?.project && ![...runs.values()].some((x) => x.project)) exportStore.setMenuRun(null);
}

function onExportEvent(name: string, payload: { export_id: string }): void {
  const id = payload.export_id;
  if (!runs.has(id)) {
    // Ours once `export_start` answers (keep a few; the rest are the panel's).
    if (toast && toast.exportId === null && toast.phase === 'running') {
      const list = early.get(id) ?? [];
      if (list.length < 64) list.push({ name, payload });
      early.set(id, list);
    }
    return;
  }
  apply(name, payload);
}

function apply(name: string, payload: unknown): void {
  const p = payload as { export_id: string };
  const mine = toast && toast.exportId === p.export_id ? toast.seq : -1;
  if (name === 'export:progress') {
    const { fraction } = payload as Events['export:progress'];
    if (toast && mine === toast.seq) patch(mine, { fraction: Math.max(toast.fraction, fraction) });
    return;
  }
  finish(p.export_id);
  if (name === 'export:done')
    patch(mine, { phase: 'done', done: payload as Events['export:done'], fraction: 1 });
  else if (name === 'export:failed') {
    const f = payload as Events['export:failed'];
    patch(mine, {
      phase: 'failed',
      label: failedLabel(toast?.job ?? null),
      error: errorText(f.code),
    });
  } else if (name === 'export:canceled' && toast?.seq === mine) set(null);
}

function listen(): Promise<void> {
  listening ??= (async () => {
    for (const name of [
      'export:progress',
      'export:done',
      'export:failed',
      'export:canceled',
    ] as const)
      await onEvent(name, (p) => onExportEvent(name, p as { export_id: string }));
  })().catch(() => {
    listening = null;
  });
  return listening;
}

/** Waits until the open project's original is decoded (FR-EXP-28, 30); false if it failed. */
function originalReady(): Promise<boolean> {
  exportStore.prefetchOriginal();
  return new Promise((resolve) => {
    const check = (): boolean => {
      const o = exportStore.get().original;
      if (o.kind === 'loading') return false;
      resolve(o.kind === 'ready');
      return true;
    };
    if (check()) return;
    const off = exportStore.subscribe(() => {
      if (check()) off();
    });
  });
}

/** The panel's current grid and target (FR-EXP-30, 31): the settings the core last accepted,
 *  else the defaults (no grid, full resolution, PNG). */
function panelSettings(): ExportSettingsInput {
  return (
    exportStore.get().settings ?? {
      grid: { mode: 'px', source_px_per_square: null },
      target_px_per_square: null,
      format: 'png',
    }
  );
}

function labelOf(job: MenuJob): string {
  if (job.kind === 'canvas') return `Exporting as ${FORMAT_LABEL[job.format]}…`;
  if (job.kind === 'canvas-save') return `Saving ${fileNameOf(job.dest)}…`;
  if (job.dest === 'exports')
    return `Exporting ${fileNameOf(job.source)} as ${FORMAT_LABEL[job.format ?? 'png']}…`;
  return `Saving ${fileNameOf(job.dest)}…`;
}

/** walk-63 L1: a failed export's title says it failed, not "Saving…". */
function failedLabel(job: MenuJob | null): string {
  if (!job) return "Couldn't export";
  if (job.kind === 'canvas') return `Couldn't export as ${FORMAT_LABEL[job.format]}`;
  if (job.kind === 'file' && job.dest === 'exports')
    return `Couldn't export ${fileNameOf(job.source)}`;
  return `Couldn't save ${fileNameOf(job.dest)}`;
}

/** A menu export runs (or waits for the original) in the toast. */
function active(): boolean {
  return !!toast && (toast.phase === 'running' || toast.phase === 'loading');
}

/** walk-63 M1: a second menu export while one runs is refused here; the running toast stays
 *  and says why. True if it was refused. */
function refuseWhileActive(): boolean {
  if (!active() || !toast) return false;
  patch(toast.seq, { notice: BUSY_NOTICE });
  return true;
}

/** True if the job belongs to the open project (a pack image's Save as doesn't). */
function isProjectJob(job: MenuJob): boolean {
  if (job.kind !== 'file') return true;
  const dir = projectStore.get().project?.project_dir;
  if (!dir) return false;
  const norm = (p: string) => p.replace(/\\/g, '/').toLowerCase();
  return norm(job.source).startsWith(`${norm(dir)}/`);
}

async function run(job: MenuJob, replace = false, again = false): Promise<void> {
  // `again`: this toast's own retry once the original is decoded.
  if (!again && refuseWhileActive()) return;
  await listen();
  const at = ++seq;
  early.clear();
  set({
    seq: at,
    phase: 'running',
    label: labelOf(job),
    exportId: null,
    fraction: 0,
    canceling: false,
    exists: null,
    done: null,
    error: null,
    notice: null,
    job,
  });
  if (job.kind !== 'file' && exportStore.get().original.kind !== 'ready') {
    patch(at, { phase: 'loading', label: 'Loading original…' });
    const ready = await originalReady();
    // Dismissed while it waited: nothing starts.
    if (toast?.seq !== at) return;
    if (!ready) {
      const o = exportStore.get().original;
      patch(at, {
        phase: 'failed',
        label: failedLabel(job),
        error: o.kind === 'failed' ? o.message : errorText('io'),
      });
      return;
    }
    patch(at, { phase: 'running', label: labelOf(job) });
  }
  let exportId: string;
  try {
    const out =
      job.kind === 'canvas'
        ? await api.export.startAs(panelSettings(), job.format, replace)
        : job.kind === 'canvas-save'
          ? await api.export.saveAs(panelSettings(), job.dest)
          : await api.export.file(job.source, job.dest, job.format, replace);
    exportId = out.export_id;
  } catch (e) {
    const m = messageOf(e);
    if (m.code === 'original_unavailable' && job.kind !== 'file') {
      // Decoded meanwhile, or not after all: wait for it again, then start (FR-EXP-30).
      patch(at, { phase: 'loading', label: 'Loading original…' });
      if ((await originalReady()) && toast?.seq === at) return run(job, replace, true);
      if (toast?.seq !== at) return;
    }
    if (m.code === 'exists') {
      const d = (m.detail ?? {}) as { path?: string; bytes?: number; mtime?: string };
      patch(at, {
        phase: 'exists',
        exists: {
          name: fileNameOf(d.path ?? ''),
          bytes: d.bytes ?? 0,
          mtime: d.mtime ?? new Date().toISOString(),
        },
      });
      return;
    }
    patch(at, { phase: 'failed', label: failedLabel(job), error: m.text });
    return;
  }
  exportStore.ignore(exportId);
  const project = isProjectJob(job);
  runs.set(exportId, { project });
  if (project) exportStore.setMenuRun(() => cancelRun(exportId));
  patch(at, { exportId });
  // Events that came before the answer.
  const before = early.get(exportId) ?? [];
  early.delete(exportId);
  for (const ev of before) apply(ev.name, ev.payload);
}

async function cancelRun(exportId: string): Promise<void> {
  if (toast?.exportId === exportId) patch(toast.seq, { canceling: true });
  try {
    await api.export.cancel(exportId);
  } catch {
    // unknown_id: it already ended; its event says how.
  }
}

/** The save dialog (FR-EXP-31, 32): opens at `export_save_default`; null when canceled. */
async function pickDest(source?: string, format?: ExportFormat): Promise<string | null> {
  let defaultPath: string | undefined;
  try {
    defaultPath = (await api.export.saveDefault(source, format)).path;
  } catch {
    // No default (the file is gone, …): the dialog opens where the OS likes.
  }
  try {
    return await pickSavePath({ title: 'Save as', defaultPath });
  } catch (e) {
    const at = ++seq;
    set({
      seq: at,
      phase: 'failed',
      label: 'Save as',
      exportId: null,
      fraction: 0,
      canceling: false,
      exists: null,
      done: null,
      error: messageOf(e).text,
      notice: null,
      job: null,
    });
    return null;
  }
}

export const menuExport = {
  get: (): MenuToast | null => toast,
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** Test helper. */
  reset(): void {
    runs.clear();
    early.clear();
    seq++;
    set(null);
  },

  /** FR-EXP-30: Export as ▸ <format> on the canvas, at the panel's size. */
  canvasExportAs: (format: ExportFormat): Promise<void> => run({ kind: 'canvas', format }),

  /** FR-EXP-31: Save as… on the canvas. Cancel in the dialog does nothing. */
  async canvasSaveAs(): Promise<void> {
    if (refuseWhileActive()) return;
    const dest = await pickDest(undefined, exportStore.get().settings?.format);
    if (dest) await run({ kind: 'canvas-save', dest });
  },

  /** FR-EXP-32: Export as ▸ <format> on a file, into the project's `exports/`. */
  fileExportAs: (source: string, format: ExportFormat): Promise<void> =>
    run({ kind: 'file', source, dest: 'exports', format }),

  /** FR-EXP-32: Save as… on a file (a pack image too). */
  async fileSaveAs(source: string): Promise<void> {
    if (refuseWhileActive()) return;
    const dest = await pickDest(source);
    if (dest) await run({ kind: 'file', source, dest, format: null });
  },

  /** The toast's Replace (FR-EXP-30, 32): the same export with `replace`. */
  replace(): Promise<void> {
    const job = toast?.job;
    return job ? run(job, true) : Promise.resolve();
  },

  /** The toast's Cancel: cancels a running export, or drops the Replace question. */
  async cancel(): Promise<void> {
    if (!toast) return;
    if (toast.phase === 'running' && toast.exportId) return cancelRun(toast.exportId);
    if (toast.phase !== 'running') set(null);
  },

  dismiss(): void {
    set(null);
  },

  /** FR-PRJ-59: Open in Explorer; a failure shows in the toast. */
  async reveal(path: string): Promise<void> {
    try {
      await api.project.reveal(path);
    } catch (e) {
      const at = ++seq;
      set({
        seq: at,
        phase: 'failed',
        label: 'Open in Explorer',
        exportId: null,
        fraction: 0,
        canceling: false,
        exists: null,
        done: null,
        error: messageOf(e).text,
        notice: null,
        job: null,
      });
    }
  },
};

export function useMenuToast(): MenuToast | null {
  return useSyncExternalStore(menuExport.subscribe, menuExport.get, menuExport.get);
}
