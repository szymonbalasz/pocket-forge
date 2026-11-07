import { describe, expect, test } from 'vitest';

import type { ProviderStatus } from '../../api';
import { balanceLabel, chipOf } from './chip';

const scenario = (patch: Partial<ProviderStatus>): ProviderStatus => ({
  mode: 'scenario',
  configured: true,
  key_suffix: 'abcd',
  connectivity: 'unknown',
  store_unavailable: false,
  ...patch,
});

const known = { kind: 'known', remaining: 1234, unit: 'CU', period_end: null } as const;

describe('provider chip (spec §7)', () => {
  test('FR-PRV-07 fake mode always shows the fake badge', () => {
    const c = chipOf({ ...scenario({}), mode: 'fake' }, known);
    expect(c).toEqual({
      state: 'fake',
      label: 'Fake provider · 1,234 CU',
      linksToSettings: false,
    });
    expect(chipOf({ ...scenario({ connectivity: 'offline' }), mode: 'fake' }, null).label).toBe(
      'Fake provider · offline · unknown',
    );
  });

  test('FR-PRV-07 scenario states', () => {
    expect(chipOf(scenario({ configured: false }), null)).toMatchObject({
      state: 'not_configured',
      label: 'not configured',
      linksToSettings: true,
    });
    expect(chipOf(scenario({ connectivity: 'auth_error' }), known)).toMatchObject({
      state: 'auth_error',
      linksToSettings: true,
    });
    expect(chipOf(scenario({ connectivity: 'offline' }), known).label).toBe('offline · 1,234 CU');
    expect(chipOf(scenario({ connectivity: 'ok' }), known).label).toBe('Scenario · ok · 1,234 CU');
    expect(chipOf(scenario({}), { kind: 'unknown' }).label).toBe(
      'Scenario · configured …abcd · unknown',
    );
  });

  test('FR-PRV-22 balance label', () => {
    expect(balanceLabel(null)).toBe('unknown');
    expect(balanceLabel({ kind: 'known', remaining: 12.6, unit: 'CU', period_end: null })).toBe(
      '13 CU',
    );
  });
});
