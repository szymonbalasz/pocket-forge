import type { Feature } from '../../shared/feature';
import { registerSettingsSection } from '../../shared/settings';
import { ChatDock, ChatDockToggle } from './ChatDock';
import { ChatSettings } from './ChatSettings';
import { installChatSink } from './chatSink';
import { HelperSettings } from './HelperSettings';

// PRM (prompt-helper spec). The prompt block is rendered by RGN in the Region edit tab and by GEN
// in the New image tab, through this index.ts (ui-guidelines § Layout). The feature fills the
// `dock` slot with the prompt chat panel and adds its status-bar toggle (#61). Its Settings
// sections edit the helper definitions (FR-PRM-02/03) and the chat.
export { PromptBlock, type PromptBlockProps, type PromptRetry } from './PromptBlock';
export { installChatSink } from './chatSink';
export { PLACEHOLDER as promptPlaceholder, type PromptHost } from './logic';
export {
  loadHelpers as loadPromptHelpers,
  resetHelpersStore as resetPromptHelpers,
  useHelpers as usePromptHelpers,
} from './store';

export const promptHelperFeature: Feature = {
  id: 'prompt-helper',
  prefix: 'prm',
  slots: { dock: ChatDock },
  status: [{ id: 'chat', priority: 90, Component: ChatDockToggle }],
};

// FR-PRM-23 (slice 5): the visible tab's published context reaches the core.
installChatSink();

registerSettingsSection({
  id: 'prm-helpers',
  title: 'Prompt helpers',
  Component: HelperSettings,
});

registerSettingsSection({
  id: 'prm-chat',
  title: 'Prompt chat',
  Component: ChatSettings,
});
