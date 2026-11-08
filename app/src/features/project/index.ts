import type { Feature } from '../../shared/feature';
import { ProjectPanel } from './ProjectPanel';
import { ProjectStatus } from './ProjectStatus';

// PRJ: the left project panel and the save-state status segment (asset-project §7).
export const projectFeature: Feature = {
  id: 'project',
  prefix: 'prj',
  slots: { left: ProjectPanel },
  status: [{ id: 'save', priority: 50, grow: true, Component: ProjectStatus }],
};
