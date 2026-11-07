import type { Feature } from '../../shared/feature';
import { ProviderChip } from './ProviderChip';
import { providerStore } from './store';

export const providerFeature: Feature = {
  id: 'provider',
  prefix: 'prv',
  slot: 'status',
  Component: ProviderChip,
};

/** Opens the provider settings (for the rail's Settings entry). */
export function openProviderSettings(): void {
  providerStore.openSettings();
}
