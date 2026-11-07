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
export { editorBridge, thumbnailUrl } from './bridge';
export type { CompareSource, PickerCandidate, PickerHandlers } from './store';
