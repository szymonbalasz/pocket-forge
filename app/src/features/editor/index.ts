import type { Feature } from '../../shared/feature';
import { EditorPanel } from './EditorPanel';

export const editorFeature: Feature = {
  id: 'editor',
  prefix: 'edt',
  slots: { canvas: EditorPanel },
};
