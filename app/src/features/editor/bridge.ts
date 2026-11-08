// What other features (region-edit) drive in the editor (R16, R19): the candidate picker's data
// and handlers, and the compare source. EDT renders both in its own slots; the picker calls no
// core command itself (FR-EDT-25).
import { api } from '../../api';
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
};

/** FR-EDT-32: a picker thumbnail URL (`edt_image_open` with `max_side` 256). For the caller
 *  (RGN) to fill `thumbUrl`; the picker itself never calls the core. */
export async function thumbnailUrl(projectId: string, imageId: string): Promise<string> {
  return (await api.editor.imageOpen(projectId, imageId, 256)).url;
}

/** The validated region (R18) for RGN to fit and send; null while there is none. */
export function useEditorRegion(): RegionState | null {
  return useEditor((s) => s.region);
}
