import type { Feature } from '../../shared/feature';
import { ReferencesHome } from './ReferencesHome';

export { ReferencesBlock, type ReferencesBlockProps } from './ReferencesBlock';
export {
  referencesBlocker,
  referencesCostKey,
  referencesDraft,
  referencesForSend,
  reloadReferences,
  restoreReferences,
  settleReferences,
} from './blockStore';

// REF: the packs home and pack view in the home (canvas) slot while References is selected
// (FR-PRJ-52), and the References block RGN renders in its Generate section (R44).
export const referencesFeature: Feature = {
  id: 'references',
  prefix: 'ref',
  slots: { canvas: ReferencesHome },
};
