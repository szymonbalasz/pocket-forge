import type { ComponentType } from 'react';

/** Where the shell renders a feature's components (ui-guidelines § Layout, `shell/Layout.tsx`).
 *  `rail` section rail · `left` project panel · `toolbar` vertical strip left of the canvas ·
 *  `canvas` the editor area · `bottom` picker / iterations strip · `status` status bar ·
 *  `right` Generate / Export panel. */
export type Slot = 'rail' | 'left' | 'toolbar' | 'canvas' | 'bottom' | 'status' | 'right';

/** A UI feature as the shell sees it (one per spec: library, editor, …). A feature renders only
 *  into its own slots, never into another feature's (ui-guidelines). */
export interface Feature {
  /** Unique feature name (the folder under features/). */
  id: string;
  /** Spec prefix in lower case (`prj`, `edt`, …); its data-testids start with it. */
  prefix: string;
  /** One component per slot the feature fills. Within a slot, registry order is render order. */
  slots: Partial<Record<Slot, ComponentType>>;
}
