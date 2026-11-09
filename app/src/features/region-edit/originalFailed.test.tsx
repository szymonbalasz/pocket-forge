// R104 (FR-PRJ-42): when the background decode of the original fails, the Generate panel shows
// the readable error instead of "still loading".
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { GeneratePanel } from './GeneratePanel';
import { rgn } from './store';
import { mockCore, polyfillRadix } from './testUtils';

let fake: FakeTauri;

beforeEach(() => rgn.detach());
afterEach(() => {
  cleanup();
  rgn.detach();
  fake.uninstall();
});

test('FR-PRJ-42 R104 original-failed shows the readable error in the Generate panel', async () => {
  polyfillRadix();
  fake = mockCore();
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  const id = projectStore.get().project?.project_id;
  expect(id).toBeTruthy();
  act(() => fake.emit('project:original-failed', { project_id: 'another', code: 'corrupt' }));
  expect(screen.queryByTestId('rgn-panel-submit-error')).toBeNull();
  act(() => fake.emit('project:original-failed', { project_id: id, code: 'corrupt' }));
  expect((await screen.findByTestId('rgn-panel-submit-error')).textContent).toBe(
    'The original can’t be decoded (the file may be damaged).',
  );
  act(() => fake.emit('project:original-failed', { project_id: id, code: 'locked' }));
  expect(screen.getByTestId('rgn-panel-submit-error').textContent).toBe(
    'The original can’t be read.',
  );
});
