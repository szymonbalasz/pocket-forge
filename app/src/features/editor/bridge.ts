// What other features (region-edit) drive in the editor (R16, R19): the candidate picker's data
// and handlers, and the compare source. EDT renders both in its own slots; the picker calls no
// core command itself (FR-EDT-25).
import { api, type MaskRef } from '../../api';
import type { MaskSync } from './maskHandoff';
import {
  editorStore,
  useEditor,
  type CompareSource,
  type PickerCandidate,
  type PickerHandlers,
  type RegionState,
} from './store';

export const editorBridge = {
  /** Shows the picker strip with these candidates (null hides it). */
  setCandidates(
    candidates: PickerCandidate[] | null,
    handlers: PickerHandlers = {},
    selectedId: string | null = null,
  ): void {
    if (!candidates) {
      editorStore.set({ picker: null, compare: null });
      return;
    }
    const sel = candidates.find((c) => c.id === selectedId && c.status === 'ready') ?? null;
    editorStore.set({
      picker: { candidates, handlers, selectedId: sel?.id ?? null },
      compare: sel?.preview ?? null,
    });
  },

  /** Shows `{url, rect}` in compare (null clears it). */
  setCompare(src: CompareSource | null): void {
    editorStore.set({ compare: src });
  },

  /** BL-14: RGN's validated region (`rgn_region_fit` with its margin and model, or a Retry's
   *  region, or a restored draft's, BL-26). The outline redraws; a different inner rect is a region
   *  change (FR-EDT-20); null removes the region. */
  setRegion(region: RegionState | null): void {
    regionSink?.(region);
  },
};

let regionSink: ((r: RegionState | null) => void) | null = null;

/** The canvas binds the region setter while a project is open (null unbinds). */
export function bindRegionSink(sink: ((r: RegionState | null) => void) | null): void {
  regionSink = sink;
}

/** FR-EDT-32: a picker thumbnail URL (`edt_image_open` with `max_side` 256). For the caller
 *  (RGN) to fill `thumbUrl`; the picker itself never calls the core. */
export async function thumbnailUrl(projectId: string, imageId: string): Promise<string> {
  return (await api.editor.imageOpen(projectId, imageId, 256)).url;
}

/** The open project's mask hand-off (set by the canvas while a project is open). */
let activeSync: MaskSync | null = null;

export function setActiveMaskSync(s: MaskSync | null): void {
  activeSync = s;
}

/** R7: submits the live mask now if it changed since the last submit, so a job sends exactly what
 *  the DM sees. Resolves to the ref the draft points at (null: no mask, or read-only). Rejects
 *  with the core's error (`locked`, `conflict`, `io`) when the submit fails. */
export function flushMask(): Promise<MaskRef | null> {
  return activeSync ? activeSync.flush() : Promise.resolve(null);
}

/** The validated region (R18) for RGN to fit and send; null while there is none. */
export function useEditorRegion(): RegionState | null {
  return useEditor((s) => s.region);
}

/** Owner 2026-11-08: the live mask has paint (true), is empty (false), or EDT has no live mask
 *  (null: no canvas). RGN disables Generate with a hint while it is empty. */
export function useEditorMaskPainted(): boolean | null {
  return useEditor((s) => s.maskPainted);
}

/** The project whose saved region EDT has restored (null before): until it is the open project,
 *  `useEditorRegion` is not that project's region and must not be saved into its draft. */
export function useEditorRegionRestored(): string | null {
  return useEditor((s) => s.regionFor);
}
