// Walk D4: after a revision restore the Generate panel shows that revision's draft (FR-PRJ-23).
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { GeneratePanel } from './GeneratePanel';
import { rgn } from './store';
import { mockCore, polyfillRadix } from './testUtils';

let fake: FakeTauri;

beforeEach(() => {
  rgn.detach();
});
afterEach(() => {
  cleanup();
  rgn.detach();
  fake.uninstall();
});

test('FR-PRJ-23 Restore r0 shows r0 draft in the panel: empty prompt, random seed (D4)', async () => {
  polyfillRadix();
  fake = mockCore();
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  act(() => rgn.setRegion({ x: 400, y: 400, w: 512, h: 512 }));
  fireEvent.change(screen.getByTestId('rgn-panel-prompt'), { target: { value: 'flooded crypt' } });
  act(() => rgn.setSeedMode('pinned'));
  act(() => rgn.setSeed(42));
  expect(rgn.get().seed).toBe(42);

  await act(() => projectStore.restore('r0'));

  await waitFor(() =>
    expect((screen.getByTestId('rgn-panel-prompt') as HTMLTextAreaElement).value).toBe(''),
  );
  expect(rgn.get().seedMode).toBe('random');
  expect(rgn.get().seed).toBeNull();
});
