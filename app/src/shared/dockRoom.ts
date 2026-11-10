// The room the dock may take (#61, FR-PRM-21, walk-61 L1). The shell measures the window and the
// panes beside the canvas (rail, left panel, toolbar, right panel) and publishes how wide the dock
// can be while the canvas keeps CANVAS_MIN px. The dock slot reads it; nothing else re-renders.
import { useSyncExternalStore } from 'react';

/** The canvas keeps at least this many px beside an open dock. */
export const CANVAS_MIN = 480;

/** px the dock may take, or `null` while unmeasured (no layout yet: tests, first render). */
let room: number | null = null;
const listeners = new Set<() => void>();

export function setDockRoom(px: number | null): void {
  const next = px === null ? null : Math.floor(px);
  if (next === room) return;
  room = next;
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
const get = () => room;

export function useDockRoom(): number | null {
  return useSyncExternalStore(subscribe, get, get);
}
