// Walk D3 (RGN part): a failed draft write must not look saved (FR-PRJ-21).
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { GeneratePanel } from './GeneratePanel';
import { DRAFT_SAVE_MS, rgn } from './store';
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

test('FR-PRJ-21 a failed draft save shows in the save state; the next commit clears it', async () => {
  polyfillRadix();
  const message = 'the project folder was moved or renamed; reopen it from home';
  fake = mockCore({ failures: { project_put_draft: { code: 'not_found', message } } });
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  expect(projectStore.get().save.kind).toBe('saved');

  fireEvent.change(screen.getByTestId('rgn-panel-prompt'), { target: { value: 'flooded crypt' } });
  await waitFor(() => expect(projectStore.get().save).toMatchObject({ kind: 'failed', message }), {
    timeout: DRAFT_SAVE_MS + 2000,
  });

  // The next change saves again (the mock fails once); its commit sets the segment back.
  fireEvent.change(screen.getByTestId('rgn-panel-prompt'), { target: { value: 'flooded hall' } });
  await waitFor(() => expect(fake.callsOf('project_put_draft')).toHaveLength(2), {
    timeout: DRAFT_SAVE_MS + 2000,
  });
  act(() => fake.emit('project:committed', { seq: 2 }));
  expect(projectStore.get().save.kind).toBe('saved');
});
