// What the home slot shows while no project is open (FR-PRJ-52): PRJ's project grid for the
// selected section, or REF's packs home. PRJ's rail sets it; both homes read it. Features never
// import each other's internals, so they meet here.
import { useSyncExternalStore } from 'react';

export type HomeView = 'projects' | 'references';

let view: HomeView = 'projects';
const listeners = new Set<() => void>();

export const homeView = {
  get: (): HomeView => view,
  set(next: HomeView): void {
    if (next === view) return;
    view = next;
    for (const l of listeners) l();
  },
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
};

export function useHomeView(): HomeView {
  return useSyncExternalStore(homeView.subscribe, homeView.get, homeView.get);
}
