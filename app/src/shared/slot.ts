// Slot idle reporting (ui-guidelines § Layout). A slot component that has nothing to show (for
// example Generate and Export on Home, with no project open) reports itself idle; the shell
// collapses a slot whose components are all idle. A component that never reports counts as busy.
import { createContext, useContext, useLayoutEffect } from 'react';

/** Set by the shell around each slot component; the default does nothing (outside a slot). */
export const SlotIdleContext = createContext<(idle: boolean) => void>(() => undefined);

/** Reports whether the calling slot component is idle. Call it unconditionally, before any early
 *  `return null`. A layout effect, so the slot collapses before the first paint. */
export function useSlotIdle(idle: boolean): void {
  const report = useContext(SlotIdleContext);
  useLayoutEffect(() => {
    report(idle);
  }, [report, idle]);
  useLayoutEffect(() => () => report(false), [report]);
}
