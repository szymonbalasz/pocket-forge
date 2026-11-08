import type { Feature } from '../../shared/feature';
import { GeneratePanel } from './GeneratePanel';
import { IterationTracker } from './IterationTracker';
import { JobStatus } from './JobStatus';

export const regionEditFeature: Feature = {
  id: 'region-edit',
  prefix: 'rgn',
  slots: { right: GeneratePanel, bottom: IterationTracker },
  // After the project's save state (50), before the provider chip (90).
  status: [{ id: 'job', priority: 60, Component: JobStatus }],
};
