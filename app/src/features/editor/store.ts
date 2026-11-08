// Editor UI state shared by the editor's slot components (toolbar, canvas, status, picker):
// the one view transform (FR-EDT-06), the tool, brush settings, mask visibility and the presets'
// inputs. Session-only (KQ-16, OQ-EDT-11): nothing here is persisted.
import { useSyncExternalStore } from 'react';

import type { MaskStatus } from './maskHandoff';
import type { Size, View } from './view';

export type Tool = 'pan' | 'region' | 'brush' | 'eraser';

/** Integer image-px rectangle. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The validated region as RGN returns it (R1, R18): inner rect + context margin. */
export interface RegionState {
  inner: Rect;
  outer: Rect;
}

export interface UndoSummary {
  depth: number;
  redo: number;
  label: string | null;
  /** An entry's call is running (FR-EDT-23). */
  busy: boolean;
  /** Last failed entry's error code (FR-EDT-23). */
  error: string | null;
}

/** What compare shows (R16): an image key's URL over `rect` in image px. */
export interface CompareSource {
  url: string;
  rect: Rect;
}

export type CompareMode = 'split' | 'flip';

/** A candidate as the picker shows it (R10 UI statuses, R19). */
export interface PickerCandidate {
  id: string;
  index: number;
  status: 'pending' | 'ready' | 'invalid';
  starred: boolean;
  /** Thumbnail URL (`edt_image_open` with `max_side` 256, FR-EDT-32); none while pending. */
  thumbUrl?: string | null;
  /** RGN's baked preview, shown by compare while this candidate is selected (R16). */
  preview?: CompareSource | null;
}

export interface PickerHandlers {
  onSelect?: (id: string) => void;
  onAccept?: (id: string) => void;
  onStar?: (id: string, starred: boolean) => void;
  onDiscard?: (id: string) => void;
}

export interface PickerState {
  candidates: PickerCandidate[];
  selectedId: string | null;
  handlers: PickerHandlers;
}

export interface EditorState {
  view: View;
  viewport: Size;
  image: Size | null;
  dpr: number;
  tool: Tool;
  /** Brush diameter in image px, 1–1024 (FR-EDT-15). */
  brushSize: number;
  /** 0–1 (FR-EDT-15). */
  hardness: number;
  maskVisible: boolean;
  /** Export's px-per-square values (FR-EDT-04, FR-EXP-26); null until set. */
  pxPerSquare: { source: number | null; target: number | null };
  region: RegionState | null;
  undo: UndoSummary;
  compare: CompareSource | null;
  compareMode: CompareMode;
  /** Split divider position across the compare rect, 0–1 (FR-EDT-24, default 50 %). */
  split: number;
  /** `\` held in flip mode: show "before". */
  flipBefore: boolean;
  picker: PickerState | null;
  /** The mask hand-off state (`edt-mask-status`, FR-EDT-28/29). */
  maskStatus: MaskStatus;
  /** The original is missing (BL-10): no tiles, tools disabled until PRJ relinks it. */
  originalMissing: boolean;
}

const initial: EditorState = {
  view: { scale: 1, tx: 0, ty: 0 },
  viewport: { w: 0, h: 0 },
  image: null,
  dpr: typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1,
  tool: 'pan',
  brushSize: 64,
  hardness: 0.5,
  maskVisible: true,
  pxPerSquare: { source: null, target: null },
  region: null,
  undo: { depth: 0, redo: 0, label: null, busy: false, error: null },
  compare: null,
  compareMode: 'split',
  split: 0.5,
  flipBefore: false,
  picker: null,
  maskStatus: { kind: 'none' },
  originalMissing: false,
};

let state: EditorState = initial;
const listeners = new Set<() => void>();

export const editorStore = {
  get: (): EditorState => state,
  set(patch: Partial<EditorState> | ((s: EditorState) => Partial<EditorState>)): void {
    const p = typeof patch === 'function' ? patch(state) : patch;
    state = { ...state, ...p };
    for (const l of listeners) l();
  },
  subscribe(l: () => void): () => void {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  /** Back to defaults (project closed / tests). Keeps the DPR and export values. */
  reset(): void {
    editorStore.set({ ...initial, dpr: state.dpr, pxPerSquare: state.pxPerSquare });
  },
};

export function useEditor<T>(select: (s: EditorState) => T): T {
  return useSyncExternalStore(
    editorStore.subscribe,
    () => select(editorStore.get()),
    () => select(editorStore.get()),
  );
}

export const BRUSH_MIN = 1;
export const BRUSH_MAX = 1024;
export const clampBrush = (n: number) => Math.min(BRUSH_MAX, Math.max(BRUSH_MIN, Math.round(n)));
export const clampHardness = (h: number) => Math.min(1, Math.max(0, Math.round(h * 100) / 100));
