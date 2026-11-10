import type { Feature } from '../../shared/feature';
import { ProviderChip } from './ProviderChip';
import { providerStore } from './store';

export const providerFeature: Feature = {
  id: 'provider',
  prefix: 'prv',
  slots: {},
  // Right end of the status bar, after the project's save state (50).
  status: [{ id: 'chip', priority: 90, drop: 4, Component: ProviderChip }],
};

/** Reloads the chip's status and balance (RGN, after the provider answers again, walk D5). */
export function refreshProviderStatus(): Promise<void> {
  return providerStore.refresh();
}

/** Opens the provider settings (the rail's Settings entry). */
export function openProviderSettings(): void {
  providerStore.openSettings();
}
