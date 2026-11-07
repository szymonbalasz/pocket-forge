import type { Feature } from '../../shared/feature';
import { EditorPanel } from './EditorPanel';
import { EditorStatus } from './EditorStatus';
import { EditorToolbar } from './EditorToolbar';

export const editorFeature: Feature = {
  id: 'editor',
  prefix: 'edt',
  slots: { toolbar: EditorToolbar, canvas: EditorPanel },
  status: [{ id: 'view', priority: 10, Component: EditorStatus }],
};

// Shared with other features (FR-EDT-23, R19): the session undo stack and the candidate picker.
export { pushUndo, revisionUndoEntry } from './session';
export type { UndoEntry } from './undo';
