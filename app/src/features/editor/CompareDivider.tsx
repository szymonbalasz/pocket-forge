// The split divider (FR-EDT-24, `edt-compare-divider`): drag to move; default 50 %.
import { useRef } from 'react';

import { dividerScreen, splitFromScreen } from './compare';
import { editorStore, type CompareSource } from './store';
import type { View } from './view';

export function CompareDivider({
  view,
  src,
  split,
  container,
}: {
  view: View;
  src: CompareSource;
  split: number;
  container: React.RefObject<HTMLDivElement | null>;
}) {
  const dragging = useRef<number | null>(null);
  const d = dividerScreen(view, src.rect, split);
  const move = (e: React.PointerEvent) => {
    if (dragging.current !== e.pointerId) return;
    const r = container.current?.getBoundingClientRect();
    if (!r) return;
    editorStore.set({
      split: splitFromScreen(editorStore.get().view, src.rect, e.clientX - r.left),
    });
  };
  return (
    <div
      className="edt-compare-divider"
      data-testid="edt-compare-divider"
      data-overlay="true"
      data-split={split.toFixed(4)}
      role="slider"
      aria-label="Before / after split"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(split * 100)}
      style={{ left: d.x, top: d.y, height: d.h }}
      onPointerDown={(e) => {
        e.stopPropagation();
        dragging.current = e.pointerId;
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={move}
      onPointerUp={(e) => {
        if (dragging.current === e.pointerId) dragging.current = null;
      }}
    />
  );
}
