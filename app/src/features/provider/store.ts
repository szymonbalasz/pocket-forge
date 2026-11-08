// Provider status for the chip and the settings view. No domain logic: every change is a core
// command; the store keeps the results and refreshes on `provider:status-changed`.
import { useSyncExternalStore } from 'react';

import { api, hasCore, onEvent, type Balance, type ProviderStatus, type Spend } from '../../api';
import { onSettingsRequested } from '../../shared/settings';

export interface ProviderState {
  status: ProviderStatus | null;
  balance: Balance | null;
  /** Session spend and cap, shown on the real-provider badge (scenario mode only). */
  spend: Spend | null;
  settingsOpen: boolean;
}

let state: ProviderState = { status: null, balance: null, spend: null, settingsOpen: false };
const listeners = new Set<() => void>();
let listening = false;

function set(patch: Partial<ProviderState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export const providerStore = {
  get: (): ProviderState => state,
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  /** Reloads the status and the balance. Balance never fails (it becomes `unknown`). */
  async refresh(): Promise<void> {
    if (!hasCore()) return;
    try {
      const status = await api.provider.status();
      set({ status });
      const balance = status.configured ? await api.provider.balance() : null;
      set({ balance });
    } catch {
      // The chip keeps its last state; the next status change retries.
    }
    if (state.status?.mode === 'scenario') await providerStore.refreshSpend();
  },

  /** Reloads the session spend and cap for the real-provider badge. */
  async refreshSpend(): Promise<void> {
    try {
      set({ spend: await api.region.getSpend() });
    } catch {
      // The badge keeps "no spend cap yet" until the next refresh.
    }
  },

  /** Subscribes to status changes once. */
  async listen(): Promise<void> {
    if (listening || !hasCore()) return;
    listening = true;
    await onEvent('provider:status-changed', () => void providerStore.refresh());
    await onEvent('rgn:spend', (e) =>
      set({ spend: { spend: e.spend, cap: e.cap, threshold: state.spend?.threshold ?? null } }),
    );
  },

  openSettings(): void {
    set({ settingsOpen: true });
  },
  closeSettings(): void {
    set({ settingsOpen: false });
  },
};

// The section rail's Settings entry (PRJ) opens the provider settings.
onSettingsRequested(() => providerStore.openSettings());

export function useProviderState(): ProviderState {
  return useSyncExternalStore(providerStore.subscribe, providerStore.get);
}
