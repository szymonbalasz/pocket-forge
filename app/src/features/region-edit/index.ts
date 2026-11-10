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
  status: [{ id: 'job', priority: 60, drop: 2, Component: JobStatus }],
};

// #55: the blend instruction text, in the settings dialog.
registerSettingsSection({
  id: 'rgn-blend-instruction',
  title: 'Region edit',
  Component: BlendInstructionSettings,
});

// Shared with GEN's New image tab (new-asset-generation §6): the capability control renderer
// (Appendix A), its pure helpers, the estimate debounce, the spend confirm and the job runner's
// state (one active job per project, FR-GEN-22).
export { ControlField } from './ControlField';
export { Hint } from './Hint';
export {
  controlKind,
  costSignature,
  evaluateInput,
  fromEvaluation,
  isValueControl,
  missingRequired,
  sentValues,
  stateOf,
  withRemembered,
  type Control,
  type Values,
} from './controls';
export { EstimateDebouncer } from './estimate';
export { SpendConfirm, type SpendConfirmProps } from './ConfirmDialog';
export { withinConfirmed, type ConfirmPlan, type SpendEstimate } from './confirm';
export { amountLabel, costLabel, errorText } from './format';
export { useActiveJobId, refreshJobs, type UiError } from './store';
// FR-PRJ-55 (#61): PRJ's panel drops an image on Working through the pick path.
export { dropNewImage, PATCHES_KEPT, type DropResult } from './store';
