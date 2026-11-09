// PRM slice 3: the prompt block in the Region edit tab (prompt-helper §7, FR-PRM-04/05/07/08/10–12;
// region-edit FR-RGN-09/21 as amended by #60).
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import { api } from '../../api';
import type { FakeTauri } from '../../api/mocks/tauri';
import type { RgnMockSeed } from '../../api/mocks/rgn';
import { projectStore } from '../../store/project';
import { GeneratePanel } from './GeneratePanel';
import { IterationTracker } from './IterationTracker';
import { JobStatus } from './JobStatus';
import { rgn } from './store';
import { mockControls, mockCore, polyfillRadix } from './testUtils';

const REGION = { x: 400, y: 400, w: 512, h: 512 };
const NO_PEOPLE = 'builtin:no-people';
const TOP_DOWN = 'builtin:top-down-map';

let fake: FakeTauri;

async function setup(seed: Partial<RgnMockSeed> = {}) {
  polyfillRadix();
  fake = mockCore({ spend: { cap: 100, threshold: 10 }, ...seed });
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
      <IterationTracker />
      <JobStatus />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  act(() => rgn.setRegion(REGION));
  await waitFor(() => expect(rgn.get().fit).not.toBeNull());
  await waitFor(() => expect(screen.getAllByTestId('prm-block-helper').length).toBeGreaterThan(0));
}

const chip = (id: string) =>
  screen.getAllByTestId('prm-block-helper').find((e) => e.dataset.recordId === id)!;
const typePrompt = (text: string) =>
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: text } });

async function submit() {
  await waitFor(() => expect(rgn.get().estimate.status).toBe('ready'), { timeout: 4000 });
  fireEvent.click(screen.getByTestId('rgn-panel-submit'));
  fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
  await waitFor(() => expect(fake.callsOf('rgn_job_submit').length).toBeGreaterThan(0));
}

beforeEach(() => rgn.detach());
afterEach(() => {
  cleanup();
  rgn.detach();
});

test('FR-PRM-04 FR-PRM-11 the Region edit tab hosts the prompt block: its placeholder, Maps helpers only', async () => {
  await setup();
  const block = screen.getByTestId('prm-block');
  expect(block.dataset.host).toBe('rgn');
  expect(screen.getByTestId('prm-block-prompt').getAttribute('placeholder')).toBe(
    "Describe the result, e.g. 'a kitchen with a stone floor and a hearth'",
  );
  const shown = screen.getAllByTestId('prm-block-helper').map((e) => e.dataset.recordId);
  expect(shown).toEqual([TOP_DOWN, 'builtin:match-style', NO_PEOPLE, 'builtin:no-grid-text']);
  // The blend toggle is an RGN row inside the block.
  expect(block.contains(screen.getByTestId('rgn-panel-blend-instruction'))).toBe(true);
  // PRM slice 6: the Chat button opens the chat window.
  expect((screen.getByTestId('prm-block-chat') as HTMLButtonElement).disabled).toBe(false);
});

test('FR-PRM-05 FR-PRM-08 a tick is saved with the draft, shown in the final prompt and sent; never re-estimates', async () => {
  await setup();
  typePrompt('a stone well');
  await waitFor(() => expect(rgn.get().estimate.status).toBe('ready'), { timeout: 4000 });
  const estimates = fake.callsOf('rgn_job_estimate').length;
  fireEvent.click(chip(NO_PEOPLE));
  expect(chip(NO_PEOPLE).getAttribute('aria-pressed')).toBe('true');
  await waitFor(() =>
    expect(
      screen
        .getAllByTestId('prm-block-part')
        .map((p) => [p.dataset.source, p.dataset.recordId ?? null]),
    ).toEqual([
      ['typed', null],
      ['helper', NO_PEOPLE],
      ['blend', null],
    ]),
  );
  await waitFor(
    () =>
      expect(fake.callsOf('project_put_draft').at(-1)?.draft).toMatchObject({
        prompt_helpers: [NO_PEOPLE],
      }),
    { timeout: 4000 },
  );
  expect(fake.callsOf('rgn_job_estimate').length).toBe(estimates);
  await submit();
  expect(fake.callsOf('rgn_job_submit')[0]!.draft).toMatchObject({
    final_prompt: 'a stone well',
    prompt_helpers: [NO_PEOPLE],
  });
});

test('FR-PRM-07 "Top-down battlemap" is badged while the Maps blend instruction already says it', async () => {
  await setup();
  typePrompt('a well');
  await waitFor(() => expect(rgn.get().blendText).not.toBeNull());
  fireEvent.click(chip(TOP_DOWN));
  await waitFor(() => expect(chip(TOP_DOWN).dataset.skipped).toBe('in_blend_instruction'));
  // Blend off: sent as a helper.
  fireEvent.click(screen.getByTestId('rgn-panel-blend-instruction'));
  await waitFor(() => expect(chip(TOP_DOWN).dataset.skipped).toBeUndefined());
});

test('FR-PRM-05 reopening restores the ticks from the draft', async () => {
  await setup();
  fireEvent.click(chip(NO_PEOPLE));
  await waitFor(
    () =>
      expect(fake.callsOf('project_put_draft').at(-1)?.draft).toMatchObject({
        prompt_helpers: [NO_PEOPLE],
      }),
    { timeout: 4000 },
  );
  cleanup();
  rgn.detach();
  projectStore.reset();
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  await waitFor(() => expect(chip(NO_PEOPLE).getAttribute('aria-pressed')).toBe('true'));
  expect(rgn.get().promptHelpers).toEqual([NO_PEOPLE]);
});

test('FR-PRM-10 FR-RGN-21 retry ticks the job helpers again and badges a changed and a missing one', async () => {
  await setup();
  typePrompt('a well');
  fireEvent.click(chip(NO_PEOPLE));
  fireEvent.click(chip('builtin:no-grid-text'));
  await submit();
  act(() => mockControls().fail());
  await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());

  // Since the job: one helper's text edited, another hidden; the ticks were cleared.
  const list = await api.prompt.listHelpers();
  await act(() =>
    api.prompt.saveHelpers(
      list.map((h) =>
        h.id === NO_PEOPLE
          ? { ...h, text: 'Empty rooms.' }
          : h.id === 'builtin:no-grid-text'
            ? { ...h, hidden: true }
            : h,
      ),
    ),
  );
  act(() => rgn.setPromptHelpers([]));
  fireEvent.click(screen.getByTestId('rgn-job-retry'));
  await waitFor(() => expect(chip(NO_PEOPLE).getAttribute('aria-pressed')).toBe('true'));
  expect(rgn.get().promptHelpers).toEqual([NO_PEOPLE]);
  expect(screen.getByTestId('prm-block-helper-changed').textContent).toMatch(/^changed since /);
  expect(screen.getByTestId('prm-block-missing').textContent).toMatch(/No grid or text/);
});

test('FR-PRM-12 a read-only project shows the helpers disabled', async () => {
  await setup({ readOnly: true });
  for (const c of screen.getAllByTestId('prm-block-helper'))
    expect((c as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).readOnly).toBe(true);
});

test('FR-RGN-11 a new region starts with an empty prompt; a region change keeps it', async () => {
  await setup();
  typePrompt('a stone well');
  // A region change (move / resize, FR-EDT-20) is not a new region.
  act(() => rgn.setRegion({ ...REGION, x: 300 }));
  expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).value).toBe(
    'a stone well',
  );
  // Removing the region, then drawing a new one.
  act(() => rgn.setRegion(null));
  act(() => rgn.setRegion({ x: 100, y: 100, w: 256, h: 256 }));
  await waitFor(() => expect(rgn.get().fit).not.toBeNull());
  expect(rgn.get().prompt).toBe('');
  expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).value).toBe('');
  await waitFor(
    () =>
      expect(fake.callsOf('project_put_draft').at(-1)?.draft).toMatchObject({
        final_prompt: '',
        region: { inner: { x: 100, y: 100, w: 256, h: 256 } },
      }),
    { timeout: 4000 },
  );
});

test('FR-RGN-11 FR-PRM-06 a remembered tick of a since-hidden helper is neither shown nor sent', async () => {
  await setup();
  typePrompt('a stone well');
  fireEvent.click(chip(NO_PEOPLE));
  await waitFor(
    () =>
      expect(fake.callsOf('project_put_draft').at(-1)?.draft).toMatchObject({
        prompt_helpers: [NO_PEOPLE],
      }),
    { timeout: 4000 },
  );
  const list = await api.prompt.listHelpers();
  await act(() =>
    api.prompt.saveHelpers(list.map((h) => (h.id === NO_PEOPLE ? { ...h, hidden: true } : h))),
  );
  // Reopen: the draft still remembers the tick.
  cleanup();
  rgn.detach();
  projectStore.reset();
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  await waitFor(() => expect(screen.getAllByTestId('prm-block-helper').length).toBeGreaterThan(0));
  expect(rgn.get().promptHelpers).toEqual([NO_PEOPLE]);
  expect(screen.getAllByTestId('prm-block-helper').map((e) => e.dataset.recordId)).not.toContain(
    NO_PEOPLE,
  );
  await waitFor(() =>
    expect(screen.getAllByTestId('prm-block-part').map((p) => p.dataset.source)).not.toContain(
      'helper',
    ),
  );
});
