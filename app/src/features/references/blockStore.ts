// The References block's state (reference-packs §7, FR-REF-15..20): the draft's `references`, the
// last `ref_resolve` result and the packs for the picker. The core reconciles (FR-REF-17); this
// store only applies the user's edit, asks `ref_resolve` again and tells RGN to save the draft.
import { useSyncExternalStore } from 'react';

import {
  api,
  type DraftReferences,
  type PackCard,
  type RefResolved,
  type RefRole,
} from '../../api';
import { errorOf, projectStore } from '../../store/project';
import { slotName, slotOf, underMin } from './format';

export interface BlockState {
  projectId: string | null;
  /** The selected model, when it has reference slots (FR-REF-15); else null (hidden). */
  modelId: string | null;
  /** The draft's `references` (schema §4.5); null = REF's defaults (FR-REF-17). */
  refs: DraftReferences | null;
  resolved: RefResolved | null;
  loading: boolean;
  error: { code: string; message: string } | null;
  /** Packs for + Pack (FR-REF-06 cards). */
  packs: PackCard[] | null;
  notices: { code: string; message: string }[];
}

const initial: BlockState = {
  projectId: null,
  modelId: null,
  refs: null,
  resolved: null,
  loading: false,
  error: null,
  packs: null,
  notices: [],
};

let state: BlockState = initial;
const listeners = new Set<() => void>();
let token = 0;
/** RGN's hook: `save` = a user edit to write with the draft (FR-REF-20). */
let changed: (save: boolean) => void = () => undefined;

function set(patch: Partial<BlockState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

/** The selection as the draft stores it: the user's latest, else the core's reconciled one. */
function current(): DraftReferences {
  return state.refs ?? state.resolved?.references ?? { packs: [], items: [] };
}

let watching = false;
function watchClose(): void {
  if (watching) return;
  watching = true;
  projectStore.subscribe(() => {
    if (!projectStore.get().project && state.projectId) block.reset();
  });
}

async function resolve(): Promise<void> {
  const { modelId, projectId } = state;
  if (!modelId || !projectId) return;
  const t = ++token;
  set({ loading: true });
  try {
    const r = await api.references.resolve(modelId, state.refs);
    if (t !== token) return;
    // Keep the selection in step with what the core reconciled (FR-REF-17: kept ticks, new
    // defaults, a renamed pack's new id, missing items kept for Retry). While the draft has no
    // selection (null), REF's defaults apply and nothing is written.
    const refs = state.refs && r.references;
    set({ resolved: r, refs, loading: false, error: null, notices: r.notices });
  } catch (e) {
    if (t !== token) return;
    set({ resolved: null, loading: false, error: errorOf(e) });
  }
  changed(false);
}

/** Applies a user edit: optimistic in `resolved`, then resolved again and saved. */
function edit(next: DraftReferences, optimistic?: (r: RefResolved) => RefResolved): void {
  set({
    refs: next,
    resolved: state.resolved && optimistic ? optimistic(state.resolved) : state.resolved,
  });
  changed(true);
  void resolve();
}

export const block = {
  get: (): BlockState => state,
  subscribe(l: () => void): () => void {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  reset(): void {
    token++;
    state = initial;
    for (const l of listeners) l();
  },
  /** RGN's callback (GeneratePanel passes it). */
  onChange(cb: (save: boolean) => void): void {
    changed = cb;
  },

  /** Binds to the open project's draft (once per project; closing the project unbinds). */
  attach(projectId: string | null, refs: DraftReferences | null): void {
    watchClose();
    if (projectId === state.projectId) return;
    token++;
    set({ ...initial, projectId, refs, packs: state.packs });
  },

  /** Retry replaced the draft (FR-RGN-44, FR-REF-26): the job's selection, with its
   *  `changed since NNN` / `missing` flags, is resolved and saved with the draft. */
  restore(refs: DraftReferences | null): void {
    token++;
    set({ refs, error: null, notices: [] });
    changed(true);
    if (state.modelId) void resolve();
  },

  /** After a submit the Retry flags are history: they are cleared, and restored items that
   *  were missing leave the selection. */
  settle(): void {
    const refs = state.refs;
    if (!refs?.items.some((i) => i.retry_of !== undefined)) return;
    const missing = new Set(
      (state.resolved?.items ?? []).filter((i) => i.state === 'missing').map((i) => i.key),
    );
    edit({
      ...refs,
      items: refs.items
        .filter((i) => !(i.retry_of !== undefined && missing.has(i.key)))
        .map((i) => ({ key: i.key, on: i.on, role: i.role })),
    });
  },

  /** The model changed (null = none, or one without slots: hidden, FR-REF-15). */
  setModel(modelId: string | null): void {
    if (modelId === state.modelId) return;
    set({ modelId, resolved: modelId ? state.resolved : null, error: null, notices: [] });
    if (modelId) void resolve();
    else changed(false);
  },

  /** Tick / untick (FR-REF-18 limits are enforced by the disabled checkbox and the core). */
  toggle(key: string): void {
    const base = current();
    const items = base.items.map((i) => (i.key === key ? { ...i, on: !i.on } : i));
    edit({ ...base, items }, (r) => {
      const item = r.items.find((i) => i.key === key);
      if (!item) return r;
      const slot = slotOf(r.slots, item);
      return {
        ...r,
        items: r.items.map((i) => (i.key === key ? { ...i, on: !i.on } : i)),
        slots: r.slots.map((s) => (s === slot ? { ...s, on: s.on + (item.on ? -1 : 1) } : s)),
      };
    });
  },

  /** A role change (FR-REF-19): the item moves to that role's slot. */
  setRole(key: string, role: RefRole | null): void {
    const base = current();
    edit({ ...base, items: base.items.map((i) => (i.key === key ? { ...i, role } : i)) });
  },

  /** + Pack: picked packs keep pick order (FR-REF-16). A pack without an id can't be picked. */
  pickPack(pack: { pack_id: string | null; name: string }): void {
    const id = pack.pack_id;
    if (!id) return;
    const base = current();
    if (base.packs.some((p) => p.pack_id === id)) return;
    edit({ ...base, packs: [...base.packs, { pack_id: id, name: pack.name }] });
  },

  /** × on a pack group: its items leave the selection. */
  unpickPack(packId: string): void {
    const base = current();
    edit(
      {
        packs: base.packs.filter((p) => p.pack_id !== packId),
        items: base.items.filter((i) => !i.key.startsWith(`pack:${packId}:`)),
      },
      (r) => {
        const gone = r.items.filter((i) => i.pack_id === packId && i.on);
        return {
          ...r,
          items: r.items.filter((i) => i.pack_id !== packId),
          slots: r.slots.map((s) => ({
            ...s,
            on: s.on - gone.filter((i) => slotOf(r.slots, i) === s).length,
          })),
        };
      },
    );
  },

  async loadPacks(): Promise<void> {
    try {
      set({ packs: await api.references.listPacks() });
    } catch {
      set({ packs: state.packs ?? [] });
    }
  },

  dismissNotices(): void {
    set({ notices: [] });
  },
};

/** The draft field RGN writes (FR-REF-20): `{references}` once the draft has one or the user
 *  edited the selection; nothing while REF's defaults apply (null, FR-REF-17). */
export function referencesDraft(): { references?: DraftReferences } {
  if (!state.projectId || !state.refs) return {};
  return { references: current() };
}

/** The selection estimate and submit send (FR-RGN-41): null = REF's defaults (FR-REF-17). */
export function referencesForSend(): DraftReferences | null {
  return state.projectId ? state.refs : null;
}

/** FR-RGN-42: the ticked items of cost-affecting slots, in order. RGN re-estimates when it
 *  changes; other slots never do (FR-REF-24). */
export function referencesCostKey(): string {
  const r = state.resolved;
  if (!state.modelId || !r?.visible) return '';
  const costly = new Set(r.slots.filter((s) => s.cost_affecting).map((s) => s.name));
  return r.items
    .filter((i) => i.on && costly.has(i.slot))
    .map((i) => `${i.slot}=${i.key}`)
    .join(',');
}

/** FR-REF-18 `references_required(slot, min)`: why submit is disabled, or null. */
export function referencesBlocker(): string | null {
  const r = state.resolved;
  if (!state.modelId || !r || !r.visible || r.ready) return null;
  const under = underMin(r.slots);
  if (!under.length) return null;
  return `References: ${under.map((s) => `${slotName(s)} needs at least ${s.min}`).join(', ')}.`;
}

/** RGN's Retry (FR-RGN-44): replaces the selection with the job's. */
export const restoreReferences = (refs: DraftReferences | null): void => block.restore(refs);
/** RGN after a successful submit: clears the Retry flags. */
export const settleReferences = (): void => block.settle();

export function useBlock(): BlockState {
  return useSyncExternalStore(block.subscribe, block.get, block.get);
}
