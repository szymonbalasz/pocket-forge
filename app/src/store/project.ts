// App-wide state shared by features: the open asset project and its save status.
// Holds no domain logic: every change is a core command, and the store keeps its result.
import { useSyncExternalStore } from 'react';

import { api, ApiError, type ImportItem, type ProjectSnapshot } from '../api';

export type SaveStatus =
  | { kind: 'unsaved' }
  | { kind: 'saving' }
  | { kind: 'saved'; at: string }
  | { kind: 'unchanged' }
  | { kind: 'failed'; code: string; message: string };

export interface ProjectState {
  project: ProjectSnapshot | null;
  save: SaveStatus;
  /** Last command error worth showing in a banner. */
  error: { code: string; message: string } | null;
  busy: boolean;
}

const initial: ProjectState = {
  project: null,
  save: { kind: 'unsaved' },
  error: null,
  busy: false,
};

let state: ProjectState = initial;
const listeners = new Set<() => void>();

function set(patch: Partial<ProjectState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

function errorOf(e: unknown): { code: string; message: string } {
  return e instanceof ApiError
    ? { code: e.code, message: e.message }
    : { code: 'ui', message: String(e) };
}

function saveStatusOf(p: ProjectSnapshot): SaveStatus {
  return p.saved
    ? { kind: 'saved', at: p.revisions.at(-1)?.created_at ?? '' }
    : { kind: 'unsaved' };
}

export const projectStore = {
  get: (): ProjectState => state,
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** Test helper: back to the initial state. */
  reset(): void {
    set(initial);
  },

  async open(path: string): Promise<boolean> {
    set({ busy: true, error: null });
    try {
      const project = await api.project.open(path);
      set({ project, save: saveStatusOf(project), busy: false });
      return true;
    } catch (e) {
      set({ error: errorOf(e), busy: false });
      return false;
    }
  },

  async close(): Promise<void> {
    try {
      await api.project.close();
    } finally {
      set({ project: null, save: { kind: 'unsaved' }, error: null });
    }
  },

  /** Ctrl+S: a revision if anything changed (the first save creates the project). */
  async save(): Promise<void> {
    if (!state.project) return;
    set({ save: { kind: 'saving' } });
    try {
      const rev = await api.project.saveRevision();
      const project = await api.project.get();
      set({ project, save: { kind: 'saved', at: rev.created_at } });
    } catch (e) {
      const err = errorOf(e);
      set({
        save: err.code === 'unchanged' ? { kind: 'unchanged' } : { kind: 'failed', ...err },
      });
    }
  },

  /** Imports paths, then opens the first usable one. Returns the per-item results. */
  async importAndOpen(paths: string[]): Promise<ImportItem[]> {
    set({ busy: true, error: null });
    try {
      const { items } = await api.library.importPaths(paths);
      set({ busy: false });
      for (const item of items) {
        if (item.status !== 'skipped') {
          await projectStore.open(item.path);
          break;
        }
      }
      return items;
    } catch (e) {
      set({ error: errorOf(e), busy: false });
      return [];
    }
  },

  clearError(): void {
    set({ error: null });
  },
};

export function useProjectState(): ProjectState {
  return useSyncExternalStore(projectStore.subscribe, projectStore.get, projectStore.get);
}
