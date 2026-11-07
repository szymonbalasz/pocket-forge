import type { ComponentType } from 'react';

/** Where the shell renders a feature's component. */
export type Slot = 'sidebar' | 'main' | 'toolbar' | 'status';

/** A UI feature as the shell sees it (one per spec: library, editor, …). */
export interface Feature {
  /** Unique feature name (the folder under features/). */
  id: string;
  /** Spec prefix in lower case (`prj`, `edt`, …); its data-testids start with it. */
  prefix: string;
  slot: Slot;
  Component: ComponentType;
}
