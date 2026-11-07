// Provider status for the chip and the settings view. No domain logic: every change is a core
// command; the store keeps the results and refreshes on `provider:status-changed`.
import { useSyncExternalStore } from 'react';

import { api, hasCore, onEvent, type Balance, type ProviderStatus } from '../../api';

export interface ProviderState {
  status: ProviderStatus | null;
  balance: Balance | null;
  settingsOpen: boolean;
}

let state: ProviderState = { status: null, balance: null, settingsOpen: false };
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
  },

  /** Subscribes to status changes once. */
  async listen(): Promise<void> {
    if (listening || !hasCore()) return;
    listening = true;
    await onEvent('provider:status-changed', () => void providerStore.refresh());
  },

  openSettings(): void {
    set({ settingsOpen: true });
  },
  closeSettings(): void {
    set({ settingsOpen: false });
  },
};

export function useProviderState(): ProviderState {
  return useSyncExternalStore(providerStore.subscribe, providerStore.get);
}
