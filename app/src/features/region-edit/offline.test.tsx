// Walk D5: after offline → online the Generate panel recovers without a reopen (RGN §7 offline,
// FR-PRV-19 offline poll delay); a capability error clears the same way.
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { GeneratePanel } from './GeneratePanel';
import { PROVIDER_PROBE_MS, isOffline, rgn } from './store';
import { mockCore, polyfillRadix } from './testUtils';

type Handlers = Record<string, (i: unknown) => unknown>;
const handlers = () => (window as unknown as { __pfMockHandlers: Handlers }).__pfMockHandlers;
const offlineError = {
  code: 'provider',
  message: 'offline',
  detail: { provider: { code: 'offline' } },
};

let fake: FakeTauri;

beforeEach(() => {
  rgn.detach();
  vi.useFakeTimers({ shouldAdvanceTime: true });
});
afterEach(() => {
  cleanup();
  rgn.detach();
  fake.uninstall();
  vi.useRealTimers();
});

async function openOffline(fail: 'models' | 'caps') {
  polyfillRadix();
  fake = mockCore();
  const h = handlers();
  const online = { list: h.provider_list_models!, caps: h.provider_get_capabilities! };
  const status = h.provider_status!;
  let offline = true;
  h.provider_status = (i) =>
    offline && fail === 'models' ? { ...(status(i) as object), connectivity: 'offline' } : status(i);
  h.provider_list_models = (i) =>
    offline && fail === 'models' ? Promise.reject(offlineError) : online.list(i);
  h.provider_get_capabilities = (i) =>
    offline && fail === 'caps'
      ? Promise.reject({ code: 'provider', message: 'the provider balance is too low' })
      : online.caps(i);
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  return {
    online: () => {
      offline = false; // like a fake case switched back: the core emits nothing until a call
    },
  };
}

test('RGN §7 walk D5 offline at open → online: the models and status recover without a reopen', async () => {
  const p = await openOffline('models');
  await waitFor(() => expect(rgn.get().modelsError).not.toBeNull());
  expect(isOffline(rgn.get())).toBe(true);
  const calls = fake.callsOf('provider_list_models').length;
  // Still offline: the next probe asks again and stays offline.
  await act(() => vi.advanceTimersByTimeAsync(PROVIDER_PROBE_MS));
  await waitFor(() => expect(fake.callsOf('provider_list_models').length).toBe(calls + 1));
  expect(isOffline(rgn.get())).toBe(true);

  p.online();
  await act(() => vi.advanceTimersByTimeAsync(PROVIDER_PROBE_MS));
  await waitFor(() => expect(rgn.get().caps).not.toBeNull());
  expect(rgn.get().modelsError).toBeNull();
  expect(rgn.get().provider?.connectivity).toBe('ok');
  expect(isOffline(rgn.get())).toBe(false);
  expect(screen.getByTestId('rgn-panel-model').textContent).toContain('All types');
  // Recovered: no more probes.
  const after = fake.callsOf('provider_list_models').length;
  await act(() => vi.advanceTimersByTimeAsync(PROVIDER_PROBE_MS * 2));
  expect(fake.callsOf('provider_list_models').length).toBe(after);
});

test('RGN §7 walk D5 a capability error clears once the provider answers again', async () => {
  const p = await openOffline('caps');
  await waitFor(() => expect(rgn.get().capsError).not.toBeNull());
  p.online();
  await act(() => vi.advanceTimersByTimeAsync(PROVIDER_PROBE_MS));
  await waitFor(() => expect(rgn.get().caps).not.toBeNull());
  expect(rgn.get().capsError).toBeNull();
});

test('RGN §7 walk D5 provider:status-changed back online reloads the models at once', async () => {
  const p = await openOffline('models');
  await waitFor(() => expect(rgn.get().modelsError).not.toBeNull());
  p.online();
  act(() => fake.emit('provider:status-changed', { connectivity: 'ok' }));
  await waitFor(() => expect(rgn.get().caps).not.toBeNull());
  expect(isOffline(rgn.get())).toBe(false);
});
