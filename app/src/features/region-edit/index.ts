import type { Feature } from '../../shared/feature';
import { registerSettingsSection } from '../../shared/settings';
import { BlendInstructionSettings } from './BlendInstructionSettings';
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

// #55: the blend instruction text, in the settings dialog.
registerSettingsSection({
  id: 'rgn-blend-instruction',
  title: 'Blend instruction',
  Component: BlendInstructionSettings,
});
