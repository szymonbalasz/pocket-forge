// Library state for the rail and the home grid (asset-project §7): the selected section, the
// library config, the section's cards, and creation progress. No domain logic: every change is a
// core command (asset-project §9) and the store keeps its result.
import { useSyncExternalStore } from 'react';

import {
  api,
  ApiError,
  hasCore,
  onEvent,
  type CreateItem,
  type LibraryConfig,
  type LibraryTrashItem,
  type ProjectCard,
  type Section,
} from '../../api';
import { errorOf, projectStore } from '../../store/project';

export interface LibraryState {
  section: Section;
  config: LibraryConfig | null;
  /** The section's cards, newest first (FR-PRJ-49); null until loaded. */
  cards: ProjectCard[] | null;
  /** Recently deleted of the section, newest first (FR-PRJ-58); null until loaded. */
  trash: LibraryTrashItem[] | null;
  /** `library_unavailable` or another listing error (FR-PRJ-02). */
  listError: { code: string; message: string } | null;
  /** Creation in progress (FR-PRJ-11): `import:progress` counts. */
  creating: { done: number; total: number } | null;
  /** The last creation's per-item result (asset-project §7 summary). */
  summary: { items: CreateItem[]; canceled: boolean } | null;
  /** First run or "Choose again": the root picker is showing. */
  choosingRoot: boolean;
}

const initial: LibraryState = {
  section: 'maps',
  config: null,
  cards: null,
  trash: null,
  listError: null,
  creating: null,
  summary: null,
  choosingRoot: false,
};

let state: LibraryState = initial;
const listeners = new Set<() => void>();
let listening = false;
/** Bumped by every `refresh`, so only the newest listing lands (FR-PRJ-49). */
let refreshSeq = 0;

function set(patch: Partial<LibraryState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

function listen(): void {
  if (listening || !hasCore()) return;
  listening = true;
  const on = (p: Promise<unknown>) => p.catch(() => undefined);
  on(
    onEvent('import:progress', ({ done, total }) => {
      if (state.creating) set({ creating: { done, total } });
    }),
  );
  // A background thumbnail landed (FR-PRJ-50): reload the grid.
  on(onEvent('project:thumb-updated', () => void libraryStore.refresh()));
  // The library trash's 30-day purge (FR-PRJ-58) runs when the core starts, before this UI
  // listens: the first listing is already the purged one, so `trash:purged` isn't heard here.
  // The open project was deleted from the left panel (FR-PRJ-57): its card goes before home
  // shows, so home never asks for the thumbnail of a folder that is gone.
  projectStore.onDeleted((path) => {
    if (state.cards?.some((c) => c.path === path))
      set({ cards: state.cards.filter((c) => c.path !== path) });
  });
}

/** Base64 of raw bytes, in chunks (pasted image data, FR-PRJ-10). */
export function toBase64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

export const libraryStore = {
  get: (): LibraryState => state,
  subscribe(listener: () => void): () => void {
    listen();
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** Test helper. */
  reset(): void {
    set(initial);
  },

  /** Loads the config, then the selected section's cards. */
  async init(): Promise<void> {
    try {
      const config = await api.library.getConfig();
      set({ config, choosingRoot: !config.configured });
      if (config.configured) await libraryStore.refresh();
    } catch (e) {
      set({ listError: errorOf(e) });
    }
  },

  /** Reloads the selected section's cards (FR-PRJ-49) and its Recently deleted (FR-PRJ-58). */
  async refresh(): Promise<void> {
    const section = state.section;
    // The newest listing wins: a late one (a thumbnail refresh) never replaces a newer one.
    const seq = ++refreshSeq;
    const current = () => seq === refreshSeq && state.section === section;
    // The grid never waits for, or fails with, the trash list.
    const trash = api.library.trashList(section).catch(() => [] as LibraryTrashItem[]);
    try {
      const cards = await api.library.listProjects(section);
      if (current()) set({ cards, listError: null });
    } catch (e) {
      if (current()) set({ cards: [], listError: errorOf(e) });
    }
    const items = await trash;
    if (current()) set({ trash: items });
  },

  /** Rail: picks a section. With a project open, it closes first (back to the home grid). */
  async setSection(section: Section): Promise<void> {
    if (projectStore.get().project) {
      await projectStore.close();
      if (projectStore.get().project) return; // close refused (`unsaved`)
    }
    if (section === state.section && state.cards) return;
    set({ section, cards: null, trash: null, listError: null, summary: null });
    await libraryStore.refresh();
  },

  /** FR-PRJ-01: sets the library root. Returns the field error to show, or null. */
  async setRoot(path: string): Promise<string | null> {
    try {
      const config = await api.library.setRoots([path.trim()]);
      set({ config, choosingRoot: false });
      await libraryStore.refresh();
      return null;
    } catch (e) {
      return errorOf(e).message;
    }
  },

  chooseRoot(on: boolean): void {
    set({ choosingRoot: on });
  },

  /** New from image (FR-PRJ-03..08, 11): paths from a drop, + New or a pasted file list. */
  async create(paths: string[]): Promise<void> {
    if (paths.length === 0 || state.creating) return;
    set({ creating: { done: 0, total: paths.length }, summary: null });
    try {
      const { items } = await api.project.create(state.section, paths);
      set({ summary: { items, canceled: false } });
    } catch (e) {
      // `canceled` keeps the projects already finished (FR-PRJ-11).
      const items =
        e instanceof ApiError && e.code === 'canceled'
          ? ((e.detail as { items?: CreateItem[] } | undefined)?.items ?? [])
          : null;
      if (items) set({ summary: { items, canceled: true } });
      else projectStore.setError(errorOf(e));
    } finally {
      set({ creating: null });
    }
    await libraryStore.refresh();
  },

  /** Pasted image data → a `pasted-<time>` project (FR-PRJ-10). */
  async createFromImage(bytes: Uint8Array): Promise<void> {
    if (state.creating) return;
    set({ creating: { done: 0, total: 1 }, summary: null });
    try {
      const { path } = await api.project.createFromClipboard(state.section, toBase64(bytes));
      set({
        summary: { items: [{ status: 'created', source: 'clipboard', path }], canceled: false },
      });
    } catch (e) {
      projectStore.setError(errorOf(e));
    } finally {
      set({ creating: null });
    }
    await libraryStore.refresh();
  },

  /** FR-PRJ-11: stops the running creation; finished projects stay. */
  async cancelCreate(): Promise<void> {
    try {
      await api.project.createCancel();
    } catch {
      // Nothing running any more.
    }
  },

  dismissSummary(): void {
    set({ summary: null });
  },

  /** FR-PRJ-45: renames a closed project's folder. Returns the error to show, or null. */
  async rename(card: ProjectCard, name: string): Promise<{ code: string; message: string } | null> {
    if (name.trim() === card.name) return null;
    try {
      await api.project.rename(card.path, name);
      await libraryStore.refresh();
      return null;
    } catch (e) {
      return errorOf(e);
    }
  },

  /** FR-PRJ-57: deletes a project from its card (the UI asked first). It moves to Recently
   *  deleted. Returns the error to show, or null. */
  async remove(card: ProjectCard): Promise<{ code: string; message: string } | null> {
    try {
      await api.project.delete(card.path);
      await libraryStore.refresh();
      return null;
    } catch (e) {
      return errorOf(e);
    }
  },

  /** FR-PRJ-58: restores a Recently deleted project to its section. */
  async restore(itemId: string): Promise<void> {
    try {
      await api.library.trashRestore(itemId);
    } catch (e) {
      projectStore.setError(errorOf(e));
    }
    await libraryStore.refresh();
  },
};

export function useLibraryState(): LibraryState {
  return useSyncExternalStore(libraryStore.subscribe, libraryStore.get, libraryStore.get);
}
