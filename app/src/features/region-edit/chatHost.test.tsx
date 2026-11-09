// PRM slice 5: the Region edit tab is the chat's host while it is visible (FR-PRM-23/43): it
// publishes the draft its estimate takes, through the shared seam and the core sink, and clears
// it when unmounted.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import { chatContext, CHAT_PUBLISH_MS } from '../../shared/chatContext';
import { projectStore } from '../../store/project';
import { installChatSink } from '../prompt-helper';
import { GeneratePanel } from './GeneratePanel';
import { rgn } from './store';
import { mockCore, polyfillRadix } from './testUtils';

let fake: FakeTauri;

beforeEach(() => {
  rgn.detach();
  chatContext.reset();
});
afterEach(() => {
  cleanup();
  rgn.detach();
  chatContext.reset();
  fake.uninstall();
});

const published = () =>
  fake.callsOf('prm_context_publish') as {
    project_id: string;
    host: string;
    input: Record<string, unknown>;
  }[];

test('FR-PRM-23 FR-PRM-43 Region edit publishes its draft for the chat and clears it when hidden', async () => {
  polyfillRadix();
  fake = mockCore();
  installChatSink();
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  const view = render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  await waitFor(() => expect(published().length).toBeGreaterThan(0), {
    timeout: CHAT_PUBLISH_MS * 10,
  });
  const first = published().at(-1)!;
  expect(first.host).toBe('rgn');
  expect(first.project_id).toBe(projectStore.get().project!.project_id);
  // The estimate's draft fields, with no region yet (FR-PRM-24's "no region yet").
  expect(first.input).toMatchObject({ region: null, blend_instruction: true });
  expect(first.input).toHaveProperty('model_id');

  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: 'flooded crypt' } });
  await waitFor(() => expect(published().at(-1)!.input.final_prompt).toBe('flooded crypt'), {
    timeout: CHAT_PUBLISH_MS * 10,
  });
  expect(chatContext.current()?.host).toBe('rgn');

  view.unmount();
  expect(fake.callsOf('prm_context_clear')).toEqual([
    { project_id: projectStore.get().project!.project_id, host: 'rgn' },
  ]);
});
