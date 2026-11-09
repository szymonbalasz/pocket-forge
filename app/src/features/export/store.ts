// Export panel state (EXP §7, §9). No domain logic: settings, plan and the export itself are core
// commands; the store keeps their results and follows the `export:*` events.
import { useSyncExternalStore } from 'react';

import {
  api,
  ApiError,
  onEvent,
  type Events,
  type ExportPhase,
  type ExportPlan,
  type ExportSettingsInput,
} from '../../api';
import { projectStore } from '../../store/project';
import {
  emptyForm,
  errorText,
  formOf,
  invalidSettingsError,
  ORIGINAL_FAILED,
  overallFraction,
  phaseWeights,
  settingsOf,
  type ExportForm,
  type FieldError,
  type OriginalState,
  type PhaseWeights,
} from './view';

export interface ExportRun {
  /** Null until `export_start` returns; events for this project's one export still apply. */
  exportId: string | null;
  phase: ExportPhase;
  /** The current phase's own fraction, as the event sent it. */
  fraction: number;
  /** The whole export's fraction (weighted phases, never goes back): the bar and the status. */
  overall: number;
  weights: PhaseWeights;
  canceling: boolean;
}

export interface ExportState {
  projectId: string | null;
  expanded: boolean;
  form: ExportForm;
  /** Settings for the form as last accepted by the core. */
  settings: ExportSettingsInput | null;
  fieldError: FieldError | null;
  plan: ExportPlan | null;
  planError: string | null;
  original: OriginalState;
  run: ExportRun | null;
  done: Events['export:done'] | null;
  error: { code: string; message: string } | null;
  /** Bumped by Ctrl+E so the panel moves focus into the section. */
  focusTick: number;
  /** The project is closing while an export runs: "Cancel the export and close?" (EXP §7). */
  closeAsk: boolean;
}

const initial: ExportState = {
  projectId: null,
  expanded: false,
  form: emptyForm,
  settings: null,
  fieldError: null,
  plan: null,
  planError: null,
  original: { kind: 'loading' },
  run: null,
  done: null,
  error: null,
  focusTick: 0,
  closeAsk: false,
};

let state: ExportState = initial;
const listeners = new Set<() => void>();
let syncSeq = 0;
let prefetched: string | null = null;
let listening: Promise<void> | null = null;
/** The closes the export question holds (R109): a second close while it's asked (← Home after
 *  the window X) joins the first instead of replacing it, so no close is ever left waiting. */
let held: { proceed: () => void; cancel: () => void }[] = [];
let guarding = false;
/** Bumped per Start and whenever a run is abandoned (project change), so a late `export_start`
 *  answer never names the wrong run. */
let runSeq = 0;
/** Exports this panel no longer follows (a closed project's): their late events are ignored even
 *  while the next run doesn't know its own id yet. */
const retired = new Set<string>();

/** PRJ close guard: hold the close while an export runs and ask first (OQ-EXP-09). */
function closeGuard(proceed: () => void, cancel: () => void): boolean {
  if (!state.run) return true;
  held.push({ proceed, cancel });
  set({ closeAsk: true, expanded: true });
  return false;
}

/** Takes every held close out (each is answered once). */
function takeHeld(): { proceed: () => void; cancel: () => void }[] {
  const all = held;
  held = [];
  return all;
}

/** The export ended (done, failed, canceled): a close it was holding no longer needs the
 *  question, so the banner goes and the close goes on (OQ-EXP-09). True if it let a close go
 *  on: the project is closing, so nothing re-plans (walk-night L4: no late `export_plan`). */
function runEnded(patch: Partial<ExportState>): boolean {
  const waiting = takeHeld();
  set({ ...patch, run: null, closeAsk: false });
  for (const w of waiting) w.proceed();
  return waiting.length > 0;
}

function set(patch: Partial<ExportState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

function codeOf(e: unknown): { code: string; message: string; field?: string; detail?: unknown } {
  return e instanceof ApiError
    ? { code: e.code, message: e.message, field: e.field, detail: e.detail }
    : { code: 'ui', message: String(e) };
}

function limitOf(detail: unknown): unknown {
  return detail && typeof detail === 'object' ? (detail as { limit?: unknown }).limit : undefined;
}

/** Applies an event only to the current export. */
function isOurs(exportId: string): boolean {
  const run = state.run;
  if (!run) return false;
  return run.exportId === null ? !retired.has(exportId) : run.exportId === exportId;
}

async function prefetch(projectId: string): Promise<void> {
  if (prefetched === projectId) return;
  prefetched = projectId;
  try {
    const r = await api.export.prefetchOriginal();
    if (state.projectId === projectId && r && r.ready === true)
      set({ original: { kind: 'ready' } });
  } catch (e) {
    if (state.projectId !== projectId) return;
    // Not for good: the next expand or Start asks again (FR-EXP-28).
    prefetched = null;
    const { code, message } = codeOf(e);
    set({ original: { kind: 'failed', message: errorText(code, message) } });
  }
}

export const exportStore = {
  get: (): ExportState => state,
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** Test helper: back to the initial state. */
  reset(): void {
    syncSeq = 0;
    runSeq++;
    retired.clear();
    prefetched = null;
    held = [];
    set(initial);
  },

  /** Subscribes to the export events once (EXP §9) and guards project close. */
  listen(): Promise<void> {
    if (!guarding) {
      guarding = true;
      projectStore.addCloseGuard(closeGuard);
      // Walk 2 N1, Keep other: the kept version's settings, so the form can't send ours back.
      projectStore.onResolved((p, keep) => {
        if (keep === 'other' && p.project_id === state.projectId) void exportStore.reloadSettings();
      });
    }
    listening ??= (async () => {
      await onEvent('project:original-ready', () => {
        if (state.projectId) set({ original: { kind: 'ready' } });
      });
      // R104: the decode failed, so original-ready won't come.
      await onEvent('project:original-failed', (p) => {
        if (state.projectId === p.project_id)
          set({ original: { kind: 'failed', message: errorText(p.code, ORIGINAL_FAILED) } });
      });
      await onEvent('export:progress', (p) => {
        const run = state.run;
        if (!isOurs(p.export_id) || !run) return;
        const overall = Math.max(run.overall, overallFraction(run.weights, p.phase, p.fraction));
        set({ run: { ...run, phase: p.phase, fraction: p.fraction, overall } });
      });
      await onEvent('export:done', (p) => {
        if (!isOurs(p.export_id)) return;
        if (!runEnded({ done: p, error: null })) void exportStore.refreshPlan();
      });
      await onEvent('export:failed', (p) => {
        if (!isOurs(p.export_id)) return;
        if (!runEnded({ error: { code: p.code, message: errorText(p.code) } }))
          void exportStore.refreshPlan();
      });
      await onEvent('export:canceled', (p) => {
        if (isOurs(p.export_id)) runEnded({});
      });
    })().catch(() => {
      listening = null;
    });
    return listening;
  },

  /** Follows the open project: loads its settings (FR-EXP-25) and the plan. */
  async bind(projectId: string | null): Promise<void> {
    if (state.projectId === projectId) return;
    syncSeq++;
    prefetched = null;
    // The old project's export (canceled by the close) is no longer ours.
    runSeq++;
    if (state.run?.exportId) retired.add(state.run.exportId);
    set({
      ...initial,
      expanded: state.expanded,
      focusTick: state.focusTick,
      projectId,
    });
    if (!projectId) return;
    if (state.expanded) void prefetch(projectId);
    try {
      const s = await api.export.getSettings();
      if (state.projectId !== projectId) return;
      set({ form: formOf(s) });
    } catch {
      // No settings yet (or EXP unavailable): keep the empty form.
    }
    // Loading isn't an edit: only plan (R101, FR-EXP-25: opening never commits).
    await exportStore.sync(false);
  },

  /** Re-reads the settings in effect (after a resolved conflict) and re-plans. */
  async reloadSettings(): Promise<void> {
    const projectId = state.projectId;
    if (!projectId) return;
    let form = emptyForm;
    try {
      form = formOf(await api.export.getSettings());
    } catch {
      // No settings: the empty form, as on open.
    }
    if (state.projectId !== projectId) return;
    set({ form });
    await exportStore.sync(false);
  },

  setExpanded(expanded: boolean): void {
    set({ expanded });
    if (expanded && state.projectId) void prefetch(state.projectId);
  },

  /** Ctrl+E: expands the section and asks the panel to focus it (EXP §7). */
  focus(): void {
    exportStore.setExpanded(true);
    set({ focusTick: state.focusTick + 1 });
  },

  /** A field changed: session settings, then the plan (FR-EXP-25: no commit per keystroke). */
  edit(patch: Partial<ExportForm>): Promise<void> {
    set({ form: { ...state.form, ...patch } });
    return exportStore.sync();
  },

  /** Sends the form to the core and refreshes the plan; the latest call wins. `push` false (the
   *  form was just loaded from the core) only plans: the core's settings are already these, and
   *  sending them would mark them for a commit (R101, FR-EXP-25). */
  async sync(push = true): Promise<void> {
    const projectId = state.projectId;
    if (!projectId) return;
    const seq = ++syncSeq;
    const parsed = settingsOf(state.form);
    if ('error' in parsed) {
      set({ fieldError: parsed.error, plan: null, planError: null });
      return;
    }
    const stale = () => seq !== syncSeq || state.projectId !== projectId;
    try {
      if (push) {
        await api.export.setSettings(parsed.settings);
        if (stale()) return;
      }
      const plan = await api.export.plan(parsed.settings);
      if (stale()) return;
      set({ settings: parsed.settings, plan, planError: null, fieldError: null });
    } catch (e) {
      if (stale()) return;
      const err = codeOf(e);
      if (err.code === 'invalid_settings' || err.code === 'invalid_input')
        set({
          fieldError: invalidSettingsError(err.field, limitOf(err.detail)),
          plan: null,
          planError: null,
        });
      else set({ plan: null, fieldError: null, planError: errorText(err.code, err.message) });
    }
  },

  /** Re-plans with the accepted settings (after an export, the file exists). */
  async refreshPlan(): Promise<void> {
    const { settings, projectId } = state;
    if (!settings || !projectId) return;
    try {
      const plan = await api.export.plan(settings);
      if (state.projectId === projectId && state.settings === settings) set({ plan });
    } catch {
      // The next edit re-plans.
    }
  },

  /** Start or Replace (FR-EXP-15, FR-EXP-21). */
  async start(): Promise<void> {
    const { settings, plan, run } = state;
    if (!settings || !plan || run) return;
    const mine = ++runSeq;
    set({
      run: {
        exportId: null,
        phase: 'compose',
        fraction: 0,
        overall: 0,
        weights: phaseWeights(plan.scale !== 1, settings.format),
        canceling: false,
      },
      done: null,
      error: null,
    });
    try {
      const { export_id } = await api.export.start(settings, plan.exists !== null);
      // Abandoned meanwhile (project change): not this panel's export any more.
      if (mine !== runSeq) {
        retired.add(export_id);
        return;
      }
      // Done or failed may already have arrived for this export.
      if (state.run && state.run.exportId === null)
        set({ run: { ...state.run, exportId: export_id } });
    } catch (e) {
      if (mine !== runSeq) return;
      const err = codeOf(e);
      set({ run: null });
      if (err.code === 'invalid_settings') {
        set({ fieldError: invalidSettingsError(err.field, limitOf(err.detail)) });
        return;
      }
      if (err.code === 'original_unavailable') {
        // Start waits for the original again: ask for it, else Loading would never end.
        set({ original: { kind: 'loading' } });
        if (state.projectId) {
          prefetched = null;
          void prefetch(state.projectId);
        }
      }
      set({ error: { code: err.code, message: errorText(err.code, err.message) } });
      if (err.code === 'exists') await exportStore.refreshPlan();
    }
  },

  /** Cancel (FR-EXP-20). `unknown_id` means it already finished; the events say how. */
  async cancel(): Promise<void> {
    const run = state.run;
    if (!run?.exportId || run.canceling) return;
    set({ run: { ...run, canceling: true } });
    try {
      await api.export.cancel(run.exportId);
    } catch {
      // unknown_id: already finished.
    }
  },

  /** Retry after an error (EXP §7). */
  retry(): Promise<void> {
    set({ error: null });
    return exportStore.start();
  },

  dismissDone(): void {
    set({ done: null });
  },

  /** "Cancel the export and close": cancels, then lets the held close go on (the core's
   *  close hook cancels too, so this never waits for `export:canceled`). */
  async confirmClose(): Promise<void> {
    const waiting = takeHeld();
    set({ closeAsk: false });
    await exportStore.cancel();
    for (const w of waiting) w.proceed();
  },

  /** "Keep exporting": the close is dropped. */
  keepExporting(): void {
    const waiting = takeHeld();
    set({ closeAsk: false });
    for (const w of waiting) w.cancel();
  },
};

export function useExportState(): ExportState {
  return useSyncExternalStore(exportStore.subscribe, exportStore.get, exportStore.get);
}
