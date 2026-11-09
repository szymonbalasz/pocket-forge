import type { Feature } from '../../shared/feature';
import { registerSettingsSection } from '../../shared/settings';
import { HelperSettings } from './HelperSettings';

// PRM (prompt-helper spec). The prompt block is rendered by RGN in the Region edit tab and by GEN
// in the New image tab, through this index.ts (ui-guidelines § Layout); the feature itself fills
// no slot. Its Settings section edits the helper definitions (FR-PRM-02/03).
export { PromptBlock, type PromptBlockProps, type PromptRetry } from './PromptBlock';
export { PLACEHOLDER as promptPlaceholder, type PromptHost } from './logic';
export {
  loadHelpers as loadPromptHelpers,
  resetHelpersStore as resetPromptHelpers,
  useHelpers as usePromptHelpers,
} from './store';

export const promptHelperFeature: Feature = {
  id: 'prompt-helper',
  prefix: 'prm',
  slots: {},
};

registerSettingsSection({
  id: 'prm-helpers',
  title: 'Prompt helpers',
  Component: HelperSettings,
});
