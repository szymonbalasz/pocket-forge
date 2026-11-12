// GEN slice 6 on the mocked core: the New image tab hosts PRM's prompt block (FR-GEN-16: typed
// prompt + ticked helpers, never the blend instruction; ticks in `draft.generate.prompt_helpers`;
// Retry badges, FR-PRM-10) and is the prompt chat's host while it is the visible tab: it
// publishes its draft and clears it on a tab switch (FR-GEN-36, FR-PRM-23).
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Tooltip } from 'radix-ui';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';

import { api } from '../../api';
import type { FakeTauri } from '../../api/mocks/tauri';
import { CHAT_PUBLISH_MS, chatContext } from '../../shared/chatContext';
import { generateTab } from '../../shared/generateTabs';
import { projectStore } from '../../store/project';
import { regionEditFeature } from '../region-edit';
import { newAssetGenerationFeature } from '.';
import { gen } from './store';
import { mockGenCore, polyfillRadix, PROJECT_DIR, rgnControls } from './testUtils';

vi.setConfig({ testTimeout: 15_000 });

const NO_PEOPLE = 'builtin:no-people';
const GeneratePanel = regionEditFeature.slots.right!;
const Tracker = regionEditFeature.slots.bottom!;
let fake: FakeTauri;

async function setup() {
  polyfillRadix();
  expect(newAssetGenerationFeature.prefix).toBe('gen');
  fake = mockGenCore();
  projectStore.reset();
  generateTab.openNextWith('new-image');
  await act(() => projectStore.open(PROJECT_DIR));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
      <Tracker />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(gen.get().fit).not.toBeNull());
  await waitFor(() => expect(screen.getAllByTestId('prm-block-helper').length).toBeGreaterThan(0));
}

const chip = (id: string) =>
  screen.getAllByTestId('prm-block-helper').find((e) => e.dataset.recordId === id)!;

const typePrompt = (text: string) =>
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: text } });

async function generate() {
  await waitFor(() => expect(gen.get().estimate.status).toBe('ready'), { timeout: 4000 });
  await act(async () => fireEvent.click(screen.getByTestId('gen-panel-submit')));
  await screen.findByTestId('rgn-confirm');
  await act(async () => fireEvent.click(screen.getByTestId('rgn-confirm-ok')));
  await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
}

beforeEach(() => projectStore.reset());
afterEach(() => {
  act(() => projectStore.reset());
  cleanup();
});

test('FR-GEN-16 FR-PRM-06 the New image tab hosts the prompt block: typed + helpers, no blend instruction', async () => {
  await setup();
  const block = screen.getByTestId('prm-block');
  expect(block.dataset.host).toBe('gen');
  typePrompt('flooded crypt');
  fireEvent.click(chip(NO_PEOPLE));
  await waitFor(() =>
    expect(screen.getByTestId('prm-block-final-prompt').textContent).toBe(
      'flooded crypt\n\nNo people, creatures or tokens.',
    ),
  );
  const previews = fake.callsOf('prm_compose_preview');
  expect(previews.at(-1)).toMatchObject({
    kind: 'map',
    typed_prompt: 'flooded crypt',
    helper_ids: [NO_PEOPLE],
    blend_instruction: null,
  });
  expect(screen.queryByTestId('rgn-panel-blend-instruction')).toBeNull();
});

test('FR-GEN-19 FR-PRM-05 FR-PRM-08 ticks are saved in draft.generate and never re-estimate', async () => {
  await setup();
  typePrompt('crypt');
  await waitFor(() => expect(gen.get().estimate.status).toBe('ready'), { timeout: 4000 });
  const estimates = fake.callsOf('gen_job_estimate').length;
  fireEvent.click(chip(NO_PEOPLE));
  await waitFor(
    () =>
      expect(
        fake
          .callsOf('project_put_draft')
          .some((c) =>
            JSON.stringify((c.draft as { generate?: unknown }).generate).includes(NO_PEOPLE),
          ),
      ).toBe(true),
    { timeout: 4000 },
  );
  const saved = fake.callsOf('project_put_draft').at(-1)!.draft as {
    generate: { prompt_helpers?: string[] };
  };
  expect(saved.generate.prompt_helpers).toEqual([NO_PEOPLE]);
  await act(() => new Promise((r) => setTimeout(r, 700)));
  expect(fake.callsOf('gen_job_estimate').length).toBe(estimates);

  await generate();
  expect(fake.callsOf('gen_job_submit').at(-1)!.draft).toMatchObject({
    typed_prompt: 'crypt',
    prompt_helpers: [NO_PEOPLE],
  });
});

test('FR-GEN-32 FR-PRM-10 Retry ticks the job helpers again and badges a changed one', async () => {
  await setup();
  typePrompt('crypt');
  fireEvent.click(chip(NO_PEOPLE));
  await generate();
  act(() => rgnControls().finish(1));
  await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());

  const list = await api.prompt.listHelpers();
  await act(() =>
    api.prompt.saveHelpers(
      list.map((h) => (h.id === NO_PEOPLE ? { ...h, text: 'Empty rooms.' } : h)),
    ),
  );
  act(() => gen.setPromptHelpers([]));
  await act(async () => fireEvent.click(screen.getByTestId('rgn-job-retry')));
  await act(async () => fireEvent.click(screen.getByTestId('rgn-job-retry-ask-confirm')));
  await waitFor(() => expect(chip(NO_PEOPLE).getAttribute('aria-pressed')).toBe('true'));
  expect(gen.get().promptHelpers).toEqual([NO_PEOPLE]);
  expect(screen.getByTestId('prm-block-helper-changed').textContent).toBe('changed since 001');
  expect(fake.callsOf('gen_job_retry_draft')).toHaveLength(1);
});

test('FR-GEN-36 FR-PRM-23 the visible tab publishes its draft; a tab switch clears it', async () => {
  const sink = { publish: vi.fn(), clear: vi.fn() };
  await setup();
  chatContext.setSink(sink);
  typePrompt('swamp');
  await waitFor(
    () =>
      expect(chatContext.current()).toMatchObject({
        host: 'gen',
        input: { model_id: 'fake-gen-wh', typed_prompt: 'swamp' },
      }),
    { timeout: CHAT_PUBLISH_MS + 2000 },
  );
  expect(sink.publish).toHaveBeenLastCalledWith(
    'gen',
    expect.objectContaining({ typed_prompt: 'swamp', size: expect.any(Object) }),
  );

  // To Region edit: the New image block unmounts and its context goes.
  fireEvent.click(screen.getAllByTestId('gen-tab')[0]!);
  await waitFor(() => expect(sink.clear).toHaveBeenCalledWith('gen'));
  expect(chatContext.current()).toBeNull();

  // Back: it publishes again.
  sink.publish.mockClear();
  fireEvent.click(screen.getAllByTestId('gen-tab')[1]!);
  await waitFor(() => expect(sink.publish).toHaveBeenCalledWith('gen', expect.anything()), {
    timeout: CHAT_PUBLISH_MS + 2000,
  });
});
