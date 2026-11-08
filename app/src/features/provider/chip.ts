// The provider status chip's label and state (provider spec §7). Pure, so it is unit tested.
import type { Balance, ProviderStatus, Spend } from '../../api';

export type ChipState = 'fake' | 'ok' | 'offline' | 'auth_error' | 'not_configured' | 'configured';

export interface Chip {
  state: ChipState;
  label: string;
  /** The chip links to the provider settings (not configured, auth error). */
  linksToSettings: boolean;
}

const units = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });

/** `1,234 CU` or `unknown`. */
export function balanceLabel(balance: Balance | null): string {
  if (!balance || balance.kind === 'unknown') return 'unknown';
  return `${units.format(balance.remaining)} ${balance.unit}`;
}

/** The always-visible real-provider badge (FR-PRV-28): real credits, and the spend cap. */
export interface RealBadge {
  label: string;
  /** No session cap yet: the first submit asks for one (FR-RGN-16). */
  capSet: boolean;
}

/** `null` in fake mode; otherwise `Real credits · cap 12 / 50 CU` or `… · no spend cap yet`. */
export function realBadgeOf(status: ProviderStatus, spend: Spend | null): RealBadge | null {
  if (status.mode !== 'scenario') return null;
  if (spend?.cap == null) {
    return { label: 'Real credits · no spend cap yet', capSet: false };
  }
  return {
    label: `Real credits · cap ${units.format(spend.spend)} / ${units.format(spend.cap)} CU`,
    capSet: true,
  };
}

export function chipOf(status: ProviderStatus, balance: Balance | null): Chip {
  const withBalance = (s: string) => `${s} · ${balanceLabel(balance)}`;
  if (status.mode === 'fake') {
    const issue =
      status.connectivity === 'offline'
        ? ' · offline'
        : status.connectivity === 'auth_error'
          ? ' · auth error'
          : '';
    return {
      state: 'fake',
      label: withBalance(`Fake provider${issue}`),
      linksToSettings: false,
    };
  }
  if (!status.configured) {
    return { state: 'not_configured', label: 'not configured', linksToSettings: true };
  }
  switch (status.connectivity) {
    case 'auth_error':
      return { state: 'auth_error', label: 'auth error', linksToSettings: true };
    case 'offline':
      return { state: 'offline', label: withBalance('offline'), linksToSettings: false };
    case 'ok':
      return { state: 'ok', label: withBalance('Scenario · ok'), linksToSettings: false };
    default:
      return {
        state: 'configured',
        label: withBalance(`Scenario · configured …${status.key_suffix ?? ''}`),
        linksToSettings: false,
      };
  }
}
