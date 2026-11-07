import type { Feature } from '../../shared/feature';
import { ProjectStatus } from './ProjectStatus';

export const projectFeature: Feature = {
  id: 'project',
  prefix: 'prj',
  slots: { status: ProjectStatus },
};
