import type { Feature } from '../../shared/feature';
import { ProviderChip } from './ProviderChip';
import { providerStore } from './store';

export const providerFeature: Feature = {
  id: 'provider',
  prefix: 'prv',
  slots: {},
  // Right end of the status bar, after the project's save state (50).
  status: [{ id: 'chip', priority: 90, Component: ProviderChip }],
};

/** Opens the provider settings (the rail's Settings entry). */
export function openProviderSettings(): void {
  providerStore.openSettings();
}
