import type { ComponentType } from 'react';

/** Where the shell renders a feature's components (ui-guidelines § Layout, `shell/Layout.tsx`).
 *  `rail` section rail · `left` project panel · `toolbar` vertical strip left of the canvas ·
 *  `canvas` the editor area · `bottom` picker / iterations strip · `right` Generate / Export panel.
 *  The status bar is not a slot: features add `status` segments to it. */
export type Slot = 'rail' | 'left' | 'toolbar' | 'canvas' | 'bottom' | 'right';

/** A status-bar segment (ui-guidelines: EDT zoom and mask, PRJ save state, RGN job state, PRV
 *  provider chip). The bar shows every registered segment, ordered by `priority` (low = left);
 *  equal priorities keep registry order. `grow` segments take the free space. */
export interface StatusSegment {
  /** Unique within the feature; the segment's wrapper gets `data-segment="<feature>:<id>"`. */
  id: string;
  priority: number;
  grow?: boolean;
  Component: ComponentType;
}

/** A UI feature as the shell sees it (one per spec: library, editor, …). A feature renders only
 *  into its own slots, never into another feature's (ui-guidelines). */
export interface Feature {
  /** Unique feature name (the folder under features/). */
  id: string;
  /** Spec prefix in lower case (`prj`, `edt`, …); its data-testids start with it. */
  prefix: string;
  /** One component per slot the feature fills. Within a slot, registry order is render order. */
  slots: Partial<Record<Slot, ComponentType>>;
  /** Status-bar segments this feature adds. */
  status?: readonly StatusSegment[];
}
