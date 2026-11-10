// The window layout (ui-guidelines § Layout, #44–#46): section rail · left panel · toolbar +
// canvas · bottom strip · status bar · dock · right panel. Each slot renders the registered
// features for it (shell/features.ts); a feature never renders into another feature's slot.
//
//   ┌────┬───────────┬─────────┬──────────────────┬──────┬─────────────┐
//   │rail│ left      │ toolbar │ canvas           │ dock │ right       │
//   │    │           ├─────────┴──────────────────┤      │             │
//   │    │           │ bottom                     │      │             │
//   │    │           │ status                     │      │             │
//   └────┴───────────┴────────────────────────────┴──────┴─────────────┘
//
// The dock (#61: the prompt chat) renders its own panel, sizes it and hides it. The shell tells it
// how wide it may get (`shared/dockRoom.ts`): the canvas keeps CANVAS_MIN px (walk-61 L1).
//
// The status bar never wraps (walk-61 L1): the `grow` segment truncates, then segments with a
// `drop` rank hide, lowest rank first, until the rest fits.
//
// The right panel collapses while every component in it reports idle (`shared/slot.ts`), e.g. on
// Home with no project open (BL-15).
import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';

import type { Feature, Slot, StatusSegment } from '../shared/feature';
import { CANVAS_MIN, setDockRoom } from '../shared/dockRoom';
import { SlotIdleContext } from '../shared/slot';

export function slotComponents(features: readonly Feature[], slot: Slot) {
  return features.flatMap((f) => {
    const C = f.slots[slot];
    return C ? [{ id: f.id, C }] : [];
  });
}

function SlotView({ features, slot }: { features: readonly Feature[]; slot: Slot }) {
  return (
    <>
      {slotComponents(features, slot).map(({ id, C }) => (
        <C key={id} />
      ))}
    </>
  );
}

/** A slot whose components report idle (`useSlotIdle`); `idle` is true once all of them are. */
function useIdleSlot(features: readonly Feature[], slot: Slot) {
  const ids = useMemo(() => slotComponents(features, slot).map((c) => c.id), [features, slot]);
  const [idleIds, setIdleIds] = useState<ReadonlySet<string>>(() => new Set());
  const reporters = useMemo(
    () =>
      new Map(
        ids.map((id) => [
          id,
          (idle: boolean) =>
            setIdleIds((prev) => {
              if (prev.has(id) === idle) return prev;
              const next = new Set(prev);
              if (idle) next.add(id);
              else next.delete(id);
              return next;
            }),
        ]),
      ),
    [ids],
  );
  const view = (
    <>
      {slotComponents(features, slot).map(({ id, C }) => (
        <SlotIdleContext.Provider key={id} value={reporters.get(id)!}>
          <C />
        </SlotIdleContext.Provider>
      ))}
    </>
  );
  return { idle: ids.length > 0 && ids.every((id) => idleIds.has(id)), view };
}

/** Every registered status segment, by priority then registry order. */
export function statusSegments(features: readonly Feature[]) {
  return features
    .flatMap((f, fi) =>
      (f.status ?? []).map((seg: StatusSegment, si) => ({
        feature: f.id,
        seg,
        order: fi * 1000 + si,
      })),
    )
    .sort((a, b) => a.seg.priority - b.seg.priority || a.order - b.order);
}

/** walk-61 L1: shows every segment, then hides `drop`-ranked ones (lowest first) while the bar
 *  overflows. Runs before paint (a resize or mutation observer, or a layout effect). */
export function fitStatusBar(bar: HTMLElement): void {
  const ranked = [...bar.querySelectorAll<HTMLElement>(':scope > [data-drop]')].sort(
    (a, b) => Number(a.dataset.drop) - Number(b.dataset.drop),
  );
  for (const el of ranked) delete el.dataset.dropped;
  for (const el of ranked) {
    if (bar.scrollWidth <= bar.clientWidth) break;
    el.dataset.dropped = 'true';
  }
}

/** The shared status bar (EDT owns it; features register segments). */
function StatusBar({ features }: { features: readonly Feature[] }) {
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const bar = ref.current;
    if (!bar) return;
    const fit = () => fitStatusBar(bar);
    fit();
    window.addEventListener('resize', fit);
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(fit);
    ro?.observe(bar);
    // A segment's text changed (a job's progress, the save state): fit again.
    const mo = typeof MutationObserver === 'undefined' ? null : new MutationObserver(fit);
    mo?.observe(bar, { childList: true, subtree: true, characterData: true });
    return () => {
      window.removeEventListener('resize', fit);
      ro?.disconnect();
      mo?.disconnect();
    };
  }, [features]);
  return (
    <footer className="pf-status" data-testid="app-status-bar" ref={ref}>
      {statusSegments(features).map(({ feature, seg }) => (
        <div
          key={`${feature}:${seg.id}`}
          className={`pf-status-segment${seg.grow ? ' pf-status-grow' : ''}`}
          data-segment={`${feature}:${seg.id}`}
          data-drop={seg.drop}
        >
          <seg.Component />
        </div>
      ))}
    </footer>
  );
}

const LEFT_MIN = 240;
const LEFT_MAX = 360;

/** Drag handle on the left panel's right edge (240–360 px). */
function useResizableWidth(initial: number) {
  const [width, setWidth] = useState(initial);
  const start = useRef<{ x: number; w: number } | null>(null);
  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      start.current = { x: e.clientX, w: width };
      (e.target as Element).setPointerCapture?.(e.pointerId);
    },
    [width],
  );
  const onPointerMove = useCallback((e: React.PointerEvent) => {
    const s = start.current;
    if (!s) return;
    setWidth(Math.min(LEFT_MAX, Math.max(LEFT_MIN, s.w + e.clientX - s.x)));
  }, []);
  const onPointerUp = useCallback(() => {
    start.current = null;
  }, []);
  return {
    width,
    handle: { onPointerDown, onPointerMove, onPointerUp, onPointerCancel: onPointerUp },
  };
}

/** walk-61 L1: publishes the dock's room (window − rail − left − toolbar − right − CANVAS_MIN),
 *  again on every window resize and whenever one of those panes changes size. */
function useDockRoomMeasure(
  root: RefObject<HTMLElement | null>,
  beside: RefObject<HTMLElement | null>[],
  key: string,
) {
  useLayoutEffect(() => {
    const measure = () => {
      const w = root.current?.getBoundingClientRect().width ?? 0;
      // No layout (tests, a hidden window): no limit beyond the dock's own.
      if (w <= 0) return setDockRoom(null);
      const taken = beside.reduce(
        (sum, r) => sum + (r.current?.getBoundingClientRect().width ?? 0),
        0,
      );
      setDockRoom(w - taken - CANVAS_MIN);
    };
    measure();
    window.addEventListener('resize', measure);
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    for (const r of [root, ...beside]) if (r.current) ro?.observe(r.current);
    return () => {
      window.removeEventListener('resize', measure);
      ro?.disconnect();
    };
    // The refs are stable; `key` says which panes exist.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
}

export function Layout({
  features,
  railFallback,
  banners,
}: {
  features: readonly Feature[];
  /** Rendered in the rail while no feature fills it (PRJ owns the rail, #45). */
  railFallback?: ReactNode;
  /** App-level banners above the work area. */
  banners?: ReactNode;
}) {
  const left = useResizableWidth(280);
  const has = (slot: Slot) => slotComponents(features, slot).length > 0;
  const right = useIdleSlot(features, 'right');
  const rootRef = useRef<HTMLDivElement>(null);
  const railRef = useRef<HTMLElement>(null);
  const leftRef = useRef<HTMLElement>(null);
  const toolbarRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLElement>(null);
  const panes = (['left', 'toolbar', 'right'] as const).filter(has).join(',');
  useDockRoomMeasure(rootRef, [railRef, leftRef, toolbarRef, rightRef], panes);

  return (
    <div className="pf-layout" data-testid="app-shell" ref={rootRef}>
      <nav className="pf-rail" data-testid="app-slot-rail" aria-label="Sections" ref={railRef}>
        {has('rail') ? <SlotView features={features} slot="rail" /> : railFallback}
      </nav>
      {has('left') && (
        <aside
          className="pf-left"
          data-testid="app-slot-left"
          style={{ width: left.width }}
          ref={leftRef}
        >
          <div className="pf-left-body">
            <SlotView features={features} slot="left" />
          </div>
          <div
            className="pf-resize"
            role="separator"
            aria-orientation="vertical"
            aria-valuemin={LEFT_MIN}
            aria-valuemax={LEFT_MAX}
            aria-valuenow={left.width}
            data-testid="app-left-resize"
            {...left.handle}
          />
        </aside>
      )}
      <div className="pf-center">
        {banners}
        <div className="pf-work">
          {has('toolbar') && (
            <div className="pf-toolbar" data-testid="app-slot-toolbar" ref={toolbarRef}>
              <SlotView features={features} slot="toolbar" />
            </div>
          )}
          <main className="pf-canvas" data-testid="app-slot-canvas">
            <SlotView features={features} slot="canvas" />
          </main>
        </div>
        {has('bottom') && (
          <div className="pf-bottom" data-testid="app-slot-bottom">
            <SlotView features={features} slot="bottom" />
          </div>
        )}
        <StatusBar features={features} />
      </div>
      {has('dock') && <SlotView features={features} slot="dock" />}
      {has('right') && (
        <aside className="pf-right" data-testid="app-slot-right" hidden={right.idle} ref={rightRef}>
          {right.view}
        </aside>
      )}
    </div>
  );
}
