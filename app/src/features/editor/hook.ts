// Wiring for the FR-EDT-26 test hook. `TEST_HOOK` is a compile-time constant: false in production
// builds, so the dynamic import below (and the hook module) is dropped from the bundle.
import type Konva from 'konva';
import { useEffect, useRef } from 'react';

import type { EditorState } from './store';
import type { TileStats } from './TileManager';

declare global {
  interface Window {
    /** Test hook (FR-EDT-26): dev and test builds only. Shape: `testHook.ts`. */
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    __pfTest?: Record<string, (...args: any[]) => any>;
  }
}

export const TEST_HOOK: boolean = import.meta.env.DEV || import.meta.env.VITE_PF_TEST === '1';

export interface TestHookSources {
  store: () => EditorState;
  stage: () => Konva.Stage | null;
  tiles: () => TileStats | null;
  /** More entries from later slices (mask readback, undo depth, …). */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  extra: () => Record<string, (...args: any[]) => any>;
}

/** Installs `window.__pfTest` while the component is mounted (test builds only). */
export function useTestHook(sources: TestHookSources): void {
  const ref = useRef(sources);
  ref.current = sources;
  useEffect(() => {
    if (!TEST_HOOK) return;
    let uninstall: (() => void) | undefined;
    let alive = true;
    void import('./testHook').then((m) => {
      if (!alive) return;
      uninstall = m.install({
        store: () => ref.current.store(),
        stage: () => ref.current.stage(),
        tiles: () => ref.current.tiles(),
        extra: () => ref.current.extra(),
      });
    });
    return () => {
      alive = false;
      uninstall?.();
    };
  }, []);
}
