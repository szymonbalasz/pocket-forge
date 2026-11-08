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
    onEvent('project:conflict', () => {
      // FR-PRJ-35: found mid-session; the snapshot's warnings now carry `conflict` too.
      if (state.project) set({ save: { kind: 'conflict' } });
      void projectStore.reload().then(() => {
        if (state.project) set({ save: { kind: 'conflict' } });
      });
    }),
  );
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

  /** Opens a project folder (FR-PRJ-14). */
  async open(path: string): Promise<boolean> {
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
  },

  /** Back to the home grid. `force` closes although a save is still pending (FR-PRJ-21). */
  async close(force = false): Promise<void> {
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

  /** After `project_resolve_conflict` (FR-PRJ-36): re-reads the project, saved again. */
  async resolved(): Promise<void> {
    if (!state.project) return;
    try {
      const project = await api.project.get();
      if (project) set({ project, save: { kind: 'saved', at: savedAt(project) } });
    } catch (e) {
      set({ error: errorOf(e) });
    }
    bump();
  },

  /** A draft write failed (RGN's `project_put_draft`): the save segment must not say Saved. The
   *  next commit sets it back (FR-PRJ-21; PRJ's own retry is separate). */
  draftSaveFailed(err: { code: string; message: string }): void {
    if (state.project)
      set({ save: { kind: 'failed', code: err.code, message: err.message, retrying: false } });
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
