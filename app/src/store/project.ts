// App-wide state shared by features: the open asset project and its save state.
// Holds no domain logic: every change is a core command, and the store keeps its result.
import { useSyncExternalStore } from 'react';

import { api, ApiError, hasCore, onEvent, type ProjectSnapshot } from '../api';

/** The PRJ status-bar segment's states (asset-project §7). */
export type SaveStatus =
  | { kind: 'saved'; at: string }
  | { kind: 'saving' }
  | { kind: 'unchanged'; at: string }
  | { kind: 'failed'; code: string; message: string; retrying: boolean }
  | { kind: 'conflict' };

export interface ProjectState {
  project: ProjectSnapshot | null;
  save: SaveStatus;
  /** Last command error worth showing in a banner. */
  error: { code: string; message: string } | null;
  busy: boolean;
  /** The project folder being opened, or the last one that failed to open (Retry). */
  opening: string | null;
  /** Bumped after every commit this UI knows of, so panels reload their lists. */
  revision: number;
}

const initial: ProjectState = {
  project: null,
  save: { kind: 'saved', at: '' },
  error: null,
  busy: false,
  opening: null,
  revision: 0,
};

let state: ProjectState = initial;
const listeners = new Set<() => void>();
let listening = false;

/** A feature's say before the open project closes, ahead of PRJ's own `unsaved` check (EXP's
 *  "Cancel the export and close?"). Returns true to allow it now; false to hold the close and
 *  call `proceed` later. */
export type CloseGuard = (proceed: () => void) => boolean;
const closeGuards = new Set<CloseGuard>();

/** Called with the snapshot after a sync conflict is resolved (FR-PRJ-36): features that keep
 *  draft state (RGN, REF, EXP) reload it from the kept version, so a later save can't write
 *  the version not kept back over it (walk 2 N1). */
export type ResolvedListener = (project: ProjectSnapshot) => void;
const resolvedListeners = new Set<ResolvedListener>();

function guardsAllow(proceed: () => void): boolean {
  for (const g of closeGuards) if (!g(proceed)) return false;
  return true;
}

function set(patch: Partial<ProjectState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export function errorOf(e: unknown): { code: string; message: string } {
  return e instanceof ApiError
    ? { code: e.code, message: e.message }
    : { code: 'ui', message: String(e) };
}

/** The save-state code of a failed save: `folder_moved` (§11, `not_found` with that
 *  `detail.reason`), else the error code. */
function saveCode(e: unknown): string {
  const reason = e instanceof ApiError ? (e.detail as { reason?: unknown } | null)?.reason : null;
  return reason === 'folder_moved' ? 'folder_moved' : errorOf(e).code;
}

function savedAt(p: ProjectSnapshot): string {
  return p.committed_at ?? p.revisions.at(-1)?.created_at ?? '';
}

/** The save segment for a freshly read snapshot: Conflict while one is open (§7), else Saved. */
function settled(p: ProjectSnapshot): SaveStatus {
  return p.warnings.some((w) => w.code === 'conflict')
    ? { kind: 'conflict' }
    : { kind: 'saved', at: savedAt(p) };
}

/** The banner text for `project:spool-dropped`, from codes only (no paths). */
export function spoolDroppedMessage(op: string, code: string): string {
  const what = op === 'add_candidates' ? 'Results of a job' : 'A job status update';
  return `${what} waiting to be saved couldn’t be applied and was discarded (${code.replace(/_/g, ' ')}).`;
}

function bump(): void {
  set({ revision: state.revision + 1 });
}

/** Core events that change the save state (asset-project §9). */
function listen(): void {
  if (listening || !hasCore()) return;
  listening = true;
  const on = (p: Promise<unknown>) => p.catch(() => undefined);
  on(
    onEvent('project:committed', () => {
      if (!state.project) return;
      set({ save: { kind: 'saved', at: new Date().toISOString() } });
      bump();
    }),
  );
  on(
    onEvent('project:save-failed', ({ reason, retrying }) => {
      if (!state.project) return;
      set({
        save: { kind: 'failed', code: reason, message: reason.replace(/_/g, ' '), retrying },
      });
    }),
  );
  on(
    onEvent('project:spool-dropped', ({ project_id, op, code }) => {
      // BL-25: a write that waited in the spool was refused for good; say so, don't drop silently.
      if (state.project?.project_id !== project_id) return;
      set({ error: { code: 'spool_dropped', message: spoolDroppedMessage(op, code) } });
      bump();
    }),
  );
  on(
    onEvent('project:conflict', () => {
      // FR-PRJ-35: found mid-session; the snapshot's warnings now carry `conflict` too.
      if (state.project) set({ save: { kind: 'conflict' } });
      void projectStore.reload().then(() => {
        if (state.project) set({ save: { kind: 'conflict' } });
      });
    }),
  );
}

/** `open` once the guards allowed it. */
async function openNow(path: string): Promise<boolean> {
  set({ busy: true, error: null, opening: path });
  try {
    if (state.project) await api.project.close().catch(() => undefined);
    const project = await api.project.open(path);
    set({ project, save: { kind: 'saved', at: savedAt(project) }, busy: false, opening: null });
    bump();
    return true;
  } catch (e) {
    set({ project: null, error: errorOf(e), busy: false });
    return false;
  }
}

/** `close` once the guards allowed it. */
async function closeNow(force: boolean): Promise<void> {
  try {
    await api.project.close(force);
  } catch (e) {
    // `unsaved` (FR-PRJ-21): stay open and say why.
    const err = errorOf(e);
    if (err.code === 'unsaved') {
      set({ error: err });
      return;
    }
  }
  set({ project: null, save: initial.save, error: null });
  bump();
}

export const projectStore = {
  get: (): ProjectState => state,
  subscribe(listener: () => void): () => void {
    listen();
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** Test helper: back to the initial state. */
  reset(): void {
    set(initial);
  },

  /** Registers a close guard (see `CloseGuard`); returns its removal. */
  addCloseGuard(guard: CloseGuard): () => void {
    closeGuards.add(guard);
    return () => closeGuards.delete(guard);
  },

  /** Opens a project folder (FR-PRJ-14). Closing the open one asks the close guards first. */
  async open(path: string): Promise<boolean> {
    if (state.project && !guardsAllow(() => void openNow(path))) return false;
    return openNow(path);
  },

  /** Back to the home grid. The close guards are asked first; `force` closes although a save is
   *  still pending (FR-PRJ-21). */
  async close(force = false): Promise<void> {
    if (!guardsAllow(() => void closeNow(force))) return;
    await closeNow(force);
  },

  /** Re-reads the open project (after a restore, relink, …). */
  async reload(): Promise<void> {
    if (!state.project) return;
    try {
      const project = await api.project.get();
      set({ project });
    } catch (e) {
      set({ error: errorOf(e) });
    }
    bump();
  },

  /** Ctrl+S: a `save` revision if anything changed since the head (FR-PRJ-23). */
  async save(label?: string): Promise<void> {
    if (!state.project) return;
    set({ save: { kind: 'saving' } });
    try {
      const rev = await api.project.saveRevision(label);
      const project = await api.project.get();
      set({ project: project ?? state.project, save: { kind: 'saved', at: rev.created_at } });
    } catch (e) {
      const err = errorOf(e);
      const at = state.project ? savedAt(state.project) : '';
      set({
        save:
          err.code === 'unchanged'
            ? { kind: 'unchanged', at }
            : err.code === 'conflict'
              ? { kind: 'conflict' }
              : { kind: 'failed', ...err, code: saveCode(e), retrying: false },
      });
    }
    bump();
  },

  /** Restores a revision as a new `restore` revision (FR-PRJ-23, 43). */
  async restore(revId: string): Promise<void> {
    if (!state.project) return;
    set({ save: { kind: 'saving' } });
    try {
      const rev = await api.project.restoreRevision(revId);
      const project = await api.project.get();
      set({ project: project ?? state.project, save: { kind: 'saved', at: rev.created_at } });
    } catch (e) {
      const code = saveCode(e);
      set({
        error: errorOf(e),
        save:
          code === 'folder_moved'
            ? { kind: 'failed', code, message: '', retrying: false }
            : { kind: 'saved', at: savedAt(state.project) },
      });
    }
    bump();
  },

  /** Registers a `ResolvedListener`; returns its removal. */
  onResolved(listener: ResolvedListener): () => void {
    resolvedListeners.add(listener);
    return () => resolvedListeners.delete(listener);
  },

  /** After `project_resolve_conflict` (FR-PRJ-36): re-reads the project and has the features
   *  reload their draft state from the kept version. */
  async resolved(): Promise<void> {
    if (!state.project) return;
    try {
      const project = await api.project.get();
      if (project) {
        set({ project, save: settled(project) });
        for (const l of resolvedListeners) l(project);
      }
    } catch (e) {
      set({ error: errorOf(e) });
    }
    bump();
  },

  /** A draft write failed (RGN's `project_put_draft`): the save segment must not say Saved. The
   *  next commit sets it back (FR-PRJ-21; PRJ's own retry is separate). */
  draftSaveFailed(e: unknown): void {
    if (!state.project) return;
    set({ save: { kind: 'failed', ...errorOf(e), code: saveCode(e), retrying: false } });
  },

  setError(error: { code: string; message: string } | null): void {
    set({ error });
  },

  clearError(): void {
    set({ error: null });
  },
};

export function useProjectState(): ProjectState {
  return useSyncExternalStore(projectStore.subscribe, projectStore.get, projectStore.get);
}
