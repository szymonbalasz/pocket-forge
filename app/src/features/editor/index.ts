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
