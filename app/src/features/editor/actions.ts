// Editor actions shared by the toolbar buttons, the status-bar presets and the shortcuts.
import { clampBrush, clampHardness, editorStore, type Tool } from './store';
import { actualScale, displayScale, fit, scaleLimits, zoomTo } from './view';

const centre = () => {
  const { viewport } = editorStore.get();
  return { x: viewport.w / 2, y: viewport.h / 2 };
};

export const actions = {
  setTool(tool: Tool): void {
    editorStore.set({ tool });
  },

  /** FR-EDT-01 / Ctrl+0. */
  fit(): void {
    const { image, viewport } = editorStore.get();
    if (!image || viewport.w <= 0 || viewport.h <= 0) return;
    editorStore.set({ view: fit(image, viewport) });
  },

  /** Ctrl+1: 1 image px = 1 device px, about the viewport centre. */
  actual(): void {
    const { image, view, dpr } = editorStore.get();
    if (!image) return;
    editorStore.set({ view: zoomTo(view, actualScale(dpr), centre()) });
  },

  /** Ctrl+2: display size (R23, CSS px). No-op until both px-per-square values exist (R6). */
  display(): void {
    const { image, view, pxPerSquare } = editorStore.get();
    const s = displayScale(pxPerSquare.source, pxPerSquare.target);
    if (!image || s === null) return;
    editorStore.set({ view: zoomTo(view, s, centre()) });
  },

  /** Clamped zoom (FR-EDT-02 limits) about the centre; used by the readout buttons. */
  zoomBy(factor: number): void {
    const { image, viewport, view, dpr } = editorStore.get();
    if (!image) return;
    const { min, max } = scaleLimits(image, viewport, dpr);
    editorStore.set({
      view: zoomTo(view, Math.min(max, Math.max(min, view.scale * factor)), centre()),
    });
  },

  /** `[` / `]`: brush size steps (≈ 10 % or at least 1 px). */
  brushSize(dir: 1 | -1): void {
    const { brushSize } = editorStore.get();
    const step = Math.max(1, Math.round(brushSize * 0.1));
    editorStore.set({ brushSize: clampBrush(brushSize + dir * step) });
  },

  /** `Shift+[` / `Shift+]`: hardness in 10 % steps. */
  hardness(dir: 1 | -1): void {
    const { hardness } = editorStore.get();
    editorStore.set({ hardness: clampHardness(hardness + dir * 0.1) });
  },

  toggleMask(): void {
    editorStore.set((s) => ({ maskVisible: !s.maskVisible }));
  },

  // Mask and undo actions are bound by the mask controller once a project is open (slice 4).
  fill: (): void => bound.fill(),
  clear: (): void => bound.clear(),
  undo: (): void => bound.undo(),
  redo: (): void => bound.redo(),
};

const noop = () => undefined;
const bound: { fill: () => void; clear: () => void; undo: () => void; redo: () => void } = {
  fill: noop,
  clear: noop,
  undo: noop,
  redo: noop,
};

/** Binds the mask / undo actions (returns the unbind function). */
export function bindActions(b: Partial<typeof bound>): () => void {
  Object.assign(bound, b);
  return () => Object.assign(bound, { fill: noop, clear: noop, undo: noop, redo: noop });
}
