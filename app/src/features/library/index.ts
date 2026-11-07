import type { Feature } from '../../shared/feature';
import { LibraryPanel } from './LibraryPanel';

export const libraryFeature: Feature = {
  id: 'library',
  prefix: 'prj',
  slot: 'sidebar',
  Component: LibraryPanel,
};
