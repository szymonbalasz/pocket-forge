import type { Feature } from '../../shared/feature';
import { EditorPicker } from './CandidatePicker';
import { EditorPanel } from './EditorPanel';
import { EditorStatus } from './EditorStatus';
import { EditorToolbar } from './EditorToolbar';

export const editorFeature: Feature = {
  id: 'editor',
  prefix: 'edt',
  slots: { toolbar: EditorToolbar, canvas: EditorPanel, bottom: EditorPicker },
  status: [{ id: 'view', priority: 10, Component: EditorStatus }],
};

// Shared with other features (FR-EDT-23, R19): the session undo stack and the candidate picker.
export { pushUndo, revisionUndoEntry } from './session';
export type { UndoEntry } from './undo';
export { editorBridge, flushMask, thumbnailUrl } from './bridge';
export type { CompareSource, PickerCandidate, PickerHandlers } from './store';
// Read-only for RGN (R18): the validated region the region tool holds, or null.
export { useEditorRegion, useEditorRegionRestored } from './bridge';
// Read-only for RGN (owner 2026-11-08): whether the live mask has paint.
export { useEditorMaskPainted } from './bridge';
export type { RegionState } from './store';
// Read-only for PRJ's conflict view (BL-22, OQ-PRJ-11 A): draws a given layer stack.
export { LayerStack } from './LayerStack';
export type { LayerStackProps } from './LayerStack';
