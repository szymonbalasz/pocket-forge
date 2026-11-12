import type { Feature } from '../../shared/feature';
import { DeleteProjectDialog } from './DeleteProjectDialog';
import { Home } from './Home';
import { Rail } from './Rail';

// PRJ: the section rail and the home grid (asset-project §7, #45). The home grid fills the
// canvas slot while no project is open; the editor fills it while one is.
// The delete confirmation, shared with the left panel (FR-PRJ-57).
export { DeleteProjectDialog };

export const libraryFeature: Feature = {
  id: 'library',
  prefix: 'prj',
  slots: { rail: Rail, canvas: Home },
};
