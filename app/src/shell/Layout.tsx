// The window layout (ui-guidelines § Layout, #44–#46): section rail · left panel · toolbar +
// canvas · bottom strip · status bar · right panel. Each slot renders the registered features
// for it (shell/features.ts); a feature never renders into another feature's slot.
//
//   ┌────┬───────────┬─────────┬──────────────────┬─────────────┐
//   │rail│ left      │ toolbar │ canvas           │ right       │
//   │    │           ├─────────┴──────────────────┤             │
//   │    │           │ bottom                     │             │
//   │    │           │ status                     │             │
//   └────┴───────────┴────────────────────────────┴─────────────┘
import { useCallback, useRef, useState, type ReactNode } from 'react';

import type { Feature, Slot, StatusSegment } from '../shared/feature';

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

/** The shared status bar (EDT owns it; features register segments). */
function StatusBar({ features }: { features: readonly Feature[] }) {
  return (
    <footer className="pf-status" data-testid="app-status-bar">
      {statusSegments(features).map(({ feature, seg }) => (
        <div
          key={`${feature}:${seg.id}`}
          className={`pf-status-segment${seg.grow ? ' pf-status-grow' : ''}`}
          data-segment={`${feature}:${seg.id}`}
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

  return (
    <div className="pf-layout" data-testid="app-shell">
      <nav className="pf-rail" data-testid="app-slot-rail" aria-label="Sections">
        {has('rail') ? <SlotView features={features} slot="rail" /> : railFallback}
      </nav>
      {has('left') && (
        <aside className="pf-left" data-testid="app-slot-left" style={{ width: left.width }}>
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
            <div className="pf-toolbar" data-testid="app-slot-toolbar">
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
      {has('right') && (
        <aside className="pf-right" data-testid="app-slot-right">
          <SlotView features={features} slot="right" />
        </aside>
      )}
    </div>
  );
}
