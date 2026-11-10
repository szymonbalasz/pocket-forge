// The chat dock's state (#61, FR-PRM-21): open or closed and its width, remembered per machine in
// `settings.json` `prompt_chat.dock` through `prm_chat_settings_put{dock}` (no detection, no
// event). Read once at startup from `prm_chat_status{cached: true}`, which starts no process.
import { useSyncExternalStore } from 'react';

import { api, hasCore } from '../../api';

/** FR-PRM-21: the dock's width limits and its first-run width, in px (the core checks them). */
export const DOCK_MIN = 320;
export const DOCK_MAX = 720;
export const DOCK_DEFAULT = 400;

export interface DockState {
  open: boolean;
  width: number;
}

let state: DockState = { open: false, width: DOCK_DEFAULT };
const listeners = new Set<() => void>();
let loading: Promise<void> | null = null;
/** A toggle before the stored state arrived wins over it. */
let touched = false;

function set(next: DockState): void {
  state = next;
  for (const l of listeners) l();
}

export const clampWidth = (w: number, max: number = DOCK_MAX): number =>
  Math.round(Math.min(max, Math.max(DOCK_MIN, w)));

/** walk-61 L1: the widest the dock may be now. The canvas keeps CANVAS_MIN px (`room`, measured
 *  by the shell; `null` = unmeasured), but the dock never gets narrower than DOCK_MIN: in a very
 *  small window it still opens at DOCK_MIN. */
export const dockMax = (room: number | null): number =>
  room === null ? DOCK_MAX : Math.max(DOCK_MIN, Math.min(DOCK_MAX, Math.floor(room)));

/** Reads the remembered state (once). */
export function loadDock(): Promise<void> {
  if (!hasCore()) return Promise.resolve();
  loading ??= api.chat.status(false, true).then(
    (s) => {
      if (touched || !s.dock) return;
      set({ open: s.dock.open, width: clampWidth(s.dock.width) });
    },
    () => undefined,
  );
  return loading;
}

function save(dock: Partial<DockState>): void {
  if (!hasCore()) return;
  void api.chat.settingsPut({ dock }).catch(() => undefined);
}

/** Opens or closes the dock. Closing never cancels a running turn (FR-PRM-35). */
export function setDockOpen(open: boolean): void {
  touched = true;
  if (state.open === open) return;
  set({ ...state, open });
  save({ open });
}

export function toggleDock(): void {
  setDockOpen(!state.open);
}

/** While dragging: the width on screen only, within what the window allows now (`max`). */
export function previewDockWidth(width: number, max: number = DOCK_MAX): void {
  const w = clampWidth(width, max);
  if (w !== state.width) set({ ...state, width: w });
}

/** The drag ended: remember the width. */
export function commitDockWidth(): void {
  touched = true;
  save({ width: state.width });
}

/** Test seam: back to the first-run state. */
export function resetDockStore(): void {
  state = { open: false, width: DOCK_DEFAULT };
  loading = null;
  touched = false;
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
const get = () => state;

export function useDock(): DockState {
  return useSyncExternalStore(subscribe, get, get);
}
