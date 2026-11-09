import type { Feature } from '../../shared/feature';
import { NEW_IMAGE_TAB, registerGenerateTab } from '../../shared/generateTabs';
import { NewImageTab } from './NewImageTab';
import { gen } from './store';

// GEN (new-asset-generation §7): the New image tab in the Generate area's tab strip, which RGN's
// Generate section draws (OQ-GEN-09), and the Generate new… dialog PRJ's + New menu opens
// (FR-GEN-01). The job, once submitted, lives in RGN's runner and tracker (FR-GEN-23/25).
export const newAssetGenerationFeature: Feature = {
  id: 'new-asset-generation',
  prefix: 'gen',
  slots: {},
};

registerGenerateTab({
  id: NEW_IMAGE_TAB,
  label: 'New image',
  Component: NewImageTab,
  withoutImage: true,
  retry: (jobId) => void gen.retry(jobId),
});

export { NewProjectDialog, type NewProjectDialogProps } from './NewProjectDialog';
