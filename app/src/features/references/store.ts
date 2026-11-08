// Packs state for the packs home and the pack view (reference-packs §7). No domain logic: every
// change is a REF core command (§9) and the store keeps its result.
import { useSyncExternalStore } from 'react';

import {
  api,
  hasCore,
  onEvent,
  type Pack,
  type PackAddItem,
  type PackCard,
  type RefRole,
  type RefTrashItem,
} from '../../api';
import { errorOf } from '../../store/project';
import { folderName, isImagePath } from './format';

export interface UiError {
  code: string;
  message: string;
}

export interface RefsState {
  /** Packs-home cards sorted by name (FR-REF-06); null until loaded. */
  cards: PackCard[] | null;
  listError: UiError | null;
  /** The open pack view, or null on the packs home. */
  openId: string | null;
  pack: Pack | null;
  packError: UiError | null;
  /** Recently deleted (FR-REF-12). */
  trash: RefTrashItem[] | null;
  /** `ref:add-progress` while copying (FR-REF-08). */
  adding: { done: number; total: number } | null;
  /** The last add's per-item result (shown until dismissed). */
  summary: { pack: string; items: PackAddItem[] } | null;
  /** The last failed action (rename, delete, …), shown as a banner. */
  error: UiError | null;
  /** What listings did (adopted, new id, conflict resolved, …; FR-REF-04, 05, 28), until
   *  dismissed. Each listing reports an action once, so notices from every listing are kept,
   *  whichever of two overlapping listings lands last (walk R5). */
  notices: ListNotice[];
}

export interface ListNotice {
  /** `<card id>:<code>:<message>`. */
  id: string;
  code: string;
  message: string;
}

const initial: RefsState = {
  cards: null,
  listError: null,
  openId: null,
  pack: null,
  packError: null,
  trash: null,
  adding: null,
  summary: null,
  error: null,
  notices: [],
};

/** The newest listing request wins the cards; an older answer only adds its notices. */
let listToken = 0;

let state: RefsState = initial;
const listeners = new Set<() => void>();
let listening = false;

function set(patch: Partial<RefsState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

function listen(): void {
  if (listening || !hasCore()) return;
  listening = true;
  const on = (p: Promise<unknown>) => p.catch(() => undefined);
  on(onEvent('ref:packs-changed', () => void refs.refresh()));
  on(onEvent('ref:thumb-updated', () => void refs.refreshList()));
  on(onEvent('ref:trash-purged', () => void refs.refreshTrash()));
  on(
    onEvent('ref:add-progress', ({ done, total }) => {
      if (state.adding) set({ adding: { done, total } });
    }),
  );
}

/** Runs a write; a failure lands in `error` (the core left packs unchanged, FR-REF-29). */
async function attempt<T>(run: () => Promise<T>): Promise<T | null> {
  try {
    const r = await run();
    set({ error: null });
    return r;
  } catch (e) {
    set({ error: errorOf(e) });
    return null;
  }
}

export const refs = {
  get: (): RefsState => state,
  subscribe(listener: () => void): () => void {
    listen();
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** Test helper. */
  reset(): void {
    set(initial);
  },

  /** Re-lists the cards, the trash and the open pack (FR-REF-09). */
  async refresh(): Promise<void> {
    await Promise.all([
      // A listing can move a conflict copy to the trash (FR-REF-28): the trash is listed after
      // it, so the item shows at once (walk R5).
      refs.refreshList().then(() => refs.refreshTrash()),
      state.openId ? refs.refreshPack() : null,
    ]);
  },

  async refreshList(): Promise<void> {
    const t = ++listToken;
    try {
      const cards = await api.references.listPacks();
      const fresh = cards.flatMap((c) =>
        c.notices.map((n) => ({ ...n, id: `${c.id}:${n.code}:${n.message}` })),
      );
      const notices = [
        ...state.notices,
        ...fresh.filter((n) => !state.notices.some((o) => o.id === n.id)),
      ];
      set(t === listToken ? { cards, listError: null, notices } : { notices });
    } catch (e) {
      if (t === listToken) set({ cards: state.cards ?? [], listError: errorOf(e) });
    }
  },

  async refreshTrash(): Promise<void> {
    try {
      set({ trash: await api.references.trashList() });
    } catch {
      set({ trash: state.trash ?? [] });
    }
  },

  async refreshPack(): Promise<void> {
    const id = state.openId;
    if (!id) return;
    try {
      const pack = await api.references.getPack(id);
      if (state.openId === id) set({ pack, packError: null });
    } catch (e) {
      if (state.openId === id) set({ pack: null, packError: errorOf(e) });
    }
  },

  /** Opens the pack view; it re-lists on open (FR-REF-09). */
  async open(packId: string): Promise<void> {
    set({ openId: packId, pack: null, packError: null, error: null, summary: null });
    await refs.refreshPack();
  },

  /** Back to the packs home. */
  close(): void {
    set({ openId: null, pack: null, packError: null, error: null, summary: null });
    void refs.refreshList();
  },

  /** + New pack (FR-REF-07). Returns the field error to show, or null. Opens the new pack. */
  async create(name: string): Promise<string | null> {
    try {
      const { pack } = await api.references.create(name);
      set({ error: null });
      await refs.refreshList();
      await refs.open(pack.pack_id);
      return null;
    } catch (e) {
      return errorOf(e).message;
    }
  },

  /** A folder dropped on the packs home → one pack named after it, ` (2)` when taken; its
   *  supported images are copied flat (FR-REF-07). Image files dropped here are ignored. */
  async createFromFolders(paths: string[]): Promise<void> {
    const folders = paths.filter((p) => !isImagePath(p));
    if (!folders.length) {
      set({
        error: {
          code: 'drop_folder',
          message: 'Drop a folder to create a pack, or open a pack to add images to it.',
        },
      });
      return;
    }
    for (const folder of folders) {
      set({ adding: { done: 0, total: 0 } });
      try {
        const r = await api.references.create(folderName(folder), [folder], true);
        set({ summary: { pack: r.pack.name, items: r.items }, error: null });
      } catch (e) {
        set({ error: errorOf(e) });
      }
      set({ adding: null });
    }
    await refs.refreshList();
  },

  /** Rename (FR-REF-10). Returns the field error to show, or null. */
  async rename(packId: string, name: string): Promise<string | null> {
    try {
      const pack = await api.references.rename(packId, name);
      if (state.openId === packId) set({ pack });
      await refs.refreshList();
      return null;
    } catch (e) {
      return errorOf(e).message;
    }
  },

  /** Notes (FR-REF-11): one `pack.json` write. */
  async setNotes(notes: string): Promise<void> {
    const id = state.openId;
    if (!id) return;
    const pack = await attempt(() => api.references.update(id, { notes }));
    if (pack && state.openId === id) set({ pack });
  },

  /** A per-image role (FR-REF-11). */
  async setRole(file: string, role: RefRole | null): Promise<void> {
    const id = state.openId;
    if (!id) return;
    const pack = await attempt(() => api.references.update(id, { roles: { [file]: role } }));
    if (pack && state.openId === id) set({ pack });
  },

  /** Copies sources into the open pack (FR-REF-08): drop, Add… or a pasted list. */
  async addImages(paths: string[]): Promise<void> {
    const id = state.openId;
    if (!id || !paths.length) return;
    set({ adding: { done: 0, total: paths.length }, summary: null });
    const r = await attempt(() => api.references.addImages(id, paths));
    set({ adding: null });
    if (r && state.openId === id)
      set({ pack: r.pack, summary: { pack: r.pack.name, items: r.items } });
    else await refs.refreshPack();
    void refs.refreshList();
  },

  /** Remove → References trash (FR-REF-12). */
  async removeImages(files: string[]): Promise<void> {
    const id = state.openId;
    if (!id) return;
    if (await attempt(() => api.references.removeImages(id, files))) {
      await Promise.all([refs.refreshPack(), refs.refreshTrash(), refs.refreshList()]);
    }
  },

  /** Delete → References trash (FR-REF-12); back to the packs home. */
  async deletePack(packId: string): Promise<boolean> {
    if (!(await attempt(() => api.references.delete(packId)))) return false;
    if (state.openId === packId) set({ openId: null, pack: null, summary: null });
    await Promise.all([refs.refreshList(), refs.refreshTrash()]);
    return true;
  },

  /** Restore from Recently deleted (FR-REF-12). */
  async restore(itemId: string): Promise<void> {
    if (await attempt(() => api.references.trashRestore(itemId))) await refs.refresh();
  },

  dismissNotices(ids: string[]): void {
    set({ notices: state.notices.filter((n) => !ids.includes(n.id)) });
  },

  dismissSummary(): void {
    set({ summary: null });
  },
  clearError(): void {
    set({ error: null });
  },
};

export function useRefs(): RefsState {
  return useSyncExternalStore(refs.subscribe, refs.get, refs.get);
}
