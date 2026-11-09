import { expect, test } from 'vitest';

import { installTestCore } from '../../api/mocks/testCore';
import { providerStore } from './store';

const core = installTestCore();

test('FR-PRV-22 a late balance of the previous key never replaces the new key’s', async () => {
  let suffix = 'AAAA';
  let release: () => void = () => undefined;
  let calls = 0;
  core.useHandlers({
    provider_status: () => ({
      mode: 'fake',
      configured: true,
      key_suffix: suffix,
      connectivity: 'ok',
      store_unavailable: false,
    }),
    provider_balance: () => {
      calls += 1;
      if (calls === 1)
        // The old key's balance call is slow (a network call).
        return new Promise<void>((r) => (release = r)).then(() => ({
          kind: 'known',
          remaining: 100,
          unit: 'CU',
          period_end: null,
        }));
      return { kind: 'known', remaining: 5, unit: 'CU', period_end: null };
    },
  });
  const first = providerStore.refresh();
  await new Promise((r) => setTimeout(r, 10));
  suffix = 'BBBB'; // a new key was saved; the settings view refreshes
  await providerStore.refresh();
  expect(providerStore.get().balance).toMatchObject({ remaining: 5 });
  release();
  await first;
  expect(providerStore.get().status?.key_suffix).toBe('BBBB');
  expect(providerStore.get().balance).toMatchObject({ remaining: 5 });
});
