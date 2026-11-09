// The helper definitions (FR-PRM-02), shared by every prompt block and the Settings section:
// loaded once with `prm_helpers_list`, replaced by each `prm:helpers-changed` (save, reset).
import { useEffect, useSyncExternalStore } from 'react';

import { api, hasCore, onEvent, type PromptHelper } from '../../api';

export interface HelpersState {
  helpers: PromptHelper[] | null;
  error: { code: string; message: string } | null;
}

let state: HelpersState = { helpers: null, error: null };
const listeners = new Set<() => void>();
let loading: Promise<void> | null = null;
let watching = false;

function set(next: HelpersState): void {
  state = next;
  for (const l of listeners) l();
}

/** Replaces the list (a save or reset result, or the event). */
export function setHelpers(helpers: PromptHelper[]): void {
  set({ helpers, error: null });
}

/** Loads the list (once; `force` reloads). */
export function loadHelpers(force = false): Promise<void> {
  if (!hasCore()) return Promise.resolve();
  if (!watching) {
    watching = true;
    void onEvent('prm:helpers-changed', (list) => setHelpers(list));
  }
  if (loading && !force) return loading;
  loading = api.prompt.listHelpers().then(setHelpers, (e: unknown) => {
    loading = null;
    const err = e as { code?: string; message?: string };
    set({
      helpers: state.helpers,
      error: { code: err.code ?? 'error', message: err.message ?? String(e) },
    });
  });
  return loading;
}

/** Test seam: forget the loaded list and the event subscription. */
export function resetHelpersStore(): void {
  state = { helpers: null, error: null };
  loading = null;
  watching = false;
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
const get = () => state;

/** The helper definitions; loads them on first use. */
export function useHelpers(): HelpersState {
  useEffect(() => {
    void loadHelpers();
  }, []);
  return useSyncExternalStore(subscribe, get, get);
}
