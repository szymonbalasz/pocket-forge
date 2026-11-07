import type { Feature } from '../../shared/feature';
import { EditorPanel } from './EditorPanel';

export const editorFeature: Feature = {
  id: 'editor',
  prefix: 'edt',
  slot: 'main',
  Component: EditorPanel,
};
