import type { Feature } from '../../shared/feature';
import { Home } from './Home';
import { Rail } from './Rail';

// PRJ: the section rail and the home grid (asset-project §7, #45). The home grid fills the
// canvas slot while no project is open; the editor fills it while one is.
export const libraryFeature: Feature = {
  id: 'library',
  prefix: 'prj',
  slots: { rail: Rail, canvas: Home },
};
