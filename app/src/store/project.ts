// App-wide state shared by features: the open asset project and its save state.
// Holds no domain logic: every change is a core command, and the store keeps its result.
import { useSyncExternalStore } from 'react';

import { api, ApiError, hasCore, onEvent, type ConflictKeep, type ProjectSnapshot } from '../api';

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
  /** Walk owner-UX L3: the one discard / restore notice, shown next to where the user acted
   *  (Results, the tracker or Storage); a newer one replaces it wherever it was. */
  trashNotice: TrashNotice | null;
  /** Bumped per project folder when it may have changed (⟳, window focus, an Add), so every
   *  list of it reloads: the project panel's and New image's sketches (walk-gen L2). */
  folders: Record<string, number>;
}

export interface TrashNotice {
  at: 'results' | 'tracker' | 'storage';
  text: string;
}

const initial: ProjectState = {
  project: null,
  save: { kind: 'saved', at: '' },
  error: null,
  busy: false,
  opening: null,
  revision: 0,
  trashNotice: null,
  folders: {},
};

let state: ProjectState = initial;
const listeners = new Set<() => void>();
let listening = false;
/** A feature's say before the open project closes, ahead of PRJ's own `unsaved` check (EXP's
 *  "Cancel the export and close?", EDT's last mask submit, R55). Returns true to allow it now;
 *  false to hold the close and later call `proceed` (once) or `cancel`. */
export type CloseGuard = (proceed: () => void, cancel: () => void) => boolean;
const closeGuards = new Set<CloseGuard>();

/** Called with the snapshot after a sync conflict is resolved (FR-PRJ-36). On Keep other,
 *  features that keep draft state (RGN, REF, EXP, EDT's mask) reload it from the kept version,
 *  so a later save can't write the version not kept back over it (walk 2 N1). On Keep this
 *  their state is this machine's version already, and saves again. */
export type ResolvedListener = (project: ProjectSnapshot, keep: ConflictKeep) => void;
const resolvedListeners = new Set<ResolvedListener>();

/** Asks the guards in turn; a held guard's `proceed` asks the ones after it. Runs `go` once all
 *  allowed it, `cancel` if one cancels. */
function askGuards(go: () => void, cancel: () => void): void {
  const list = [...closeGuards];
  const from = (start: number): void => {
    for (const [i, guard] of list.entries()) {
      if (i >= start && !guard(() => from(i + 1), cancel)) return;
    }
    go();
  };
  from(0);
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

/** §11: the project folder was moved or renamed while open (`not_found` with
 *  `detail.reason = folder_moved`). */
export function isFolderMovedError(e: unknown): boolean {
  return (
    e instanceof ApiError && (e.detail as { reason?: unknown } | null)?.reason === 'folder_moved'
  );
}

/** §11: the folder-moved state features act on (the save segment failed with it, or the
 *  project's warnings carry it). It clears with the next commit. */
export function folderMoved(s: ProjectState): boolean {
  return (
    (s.save.kind === 'failed' && s.save.code === 'folder_moved') ||
    !!s.project?.warnings.some((w) => w.code === 'folder_moved')
  );
}

/** What a panel shows instead of its figures while the folder is moved (never OS text). */
export const FOLDER_MOVED_TEXT =
  'The project folder was moved or renamed outside Pocket Forge. Reopen it from home.';

/** The save-state code of a failed save: `folder_moved` (§11, `not_found` with that
 *  `detail.reason`), else the error code. */
function saveCode(e: unknown): string {
  return isFolderMovedError(e) ? 'folder_moved' : errorOf(e).code;
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
      const p = state.project;
      if (!p) return;
      // A commit means the folder is back: the core drops its folder_moved warning too.
      const warnings = p.warnings.filter((w) => w.code !== 'folder_moved');
      set({
        save: { kind: 'saved', at: new Date().toISOString() },
        ...(warnings.length !== p.warnings.length ? { project: { ...p, warnings } } : {}),
      });
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
    if (state.project) {
      // `unsaved` (FR-PRJ-21): a save is still waiting; opening would drop it, so stay and say
      // why (the core's `project_open` doesn't ask). Other refusals don't block the open.
      const refused = await api.project.close().then(
        () => null,
        (e: unknown) => errorOf(e),
      );
      if (refused?.code === 'unsaved') {
        set({ error: refused, busy: false, opening: null });
        return false;
      }
    }
    const project = await api.project.open(path);
    // A conflict found at open shows as Conflict, not Saved (walk 2 N3).
    set({ project, save: settled(project), busy: false, opening: null, trashNotice: null });
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
  set({ project: null, save: initial.save, error: null, trashNotice: null });
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
    if (!state.project) return openNow(path);
    return new Promise((resolve) =>
      askGuards(
        () => void openNow(path).then(resolve),
        () => resolve(false),
      ),
    );
  },

  /** Back to the home grid. The close guards are asked first; `force` closes although a save is
   *  still pending (FR-PRJ-21). Resolves once closed, refused or cancelled (callers check
   *  whether a project is still open). */
  async close(force = false): Promise<void> {
    return new Promise((resolve) => askGuards(() => void closeNow(force).then(resolve), resolve));
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
      if (!state.project) return; // closed meanwhile: nothing to show it on
      const err = errorOf(e);
      const at = savedAt(state.project);
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
    const before = state.save;
    set({ save: { kind: 'saving' } });
    try {
      const rev = await api.project.restoreRevision(revId);
      const project = await api.project.get();
      set({ project: project ?? state.project, save: { kind: 'saved', at: rev.created_at } });
    } catch (e) {
      if (!state.project) return; // closed meanwhile: nothing to show it on
      const err = errorOf(e);
      const code = saveCode(e);
      // A refusal that writes nothing (unknown or missing revision) leaves the segment as it
      // was; one that couldn't write never says Saved (FR-PRJ-21, §7).
      set({
        error: err,
        save:
          code === 'folder_moved'
            ? { kind: 'failed', code, message: '', retrying: false }
            : code === 'conflict'
              ? { kind: 'conflict' }
              : code === 'locked' || code === 'io'
                ? { kind: 'failed', ...err, retrying: false }
                : before.kind === 'saving'
                  ? settled(state.project)
                  : before,
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
  async resolved(keep: ConflictKeep): Promise<void> {
    if (!state.project) return;
    try {
      const project = await api.project.get();
      if (project) {
        set({ project, save: settled(project) });
        for (const l of resolvedListeners) l(project, keep);
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

  /** A feature's call failed with `folder_moved` (§11): the save segment and banner say so,
   *  and every feature sees `folderMoved` (Storage clears, Generate is blocked). */
  noteFolderMoved(): void {
    if (!state.project || folderMoved(state)) return;
    set({ save: { kind: 'failed', code: 'folder_moved', message: '', retrying: false } });
  },

  /** A project folder may have changed (⟳, focus, an Add, Explorer): its lists reload. */
  folderChanged(folder: string): void {
    set({ folders: { ...state.folders, [folder]: (state.folders[folder] ?? 0) + 1 } });
  },

  /** Shows the discard / restore notice where the user acted (replacing any other). */
  setTrashNotice(notice: TrashNotice | null): void {
    set({ trashNotice: notice });
  },

  /** Drops an open-time warning that no longer applies (RGN: `resumable` once no job is left
   *  running, walk D7). The core's list is computed at open only. */
  clearWarning(code: string): void {
    const p = state.project;
    if (!p || !p.warnings.some((w) => w.code === code)) return;
    set({ project: { ...p, warnings: p.warnings.filter((w) => w.code !== code) } });
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
