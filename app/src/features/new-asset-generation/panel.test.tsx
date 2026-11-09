// The New image tab on the mocked core (new-asset-generation §6–7, §13 Vitest): the tab strip,
// models and controls for `generate`, the Size block per mode, the sketch block, the References
// block's own draft, estimate → confirm → submit through RGN's runner, the tracker badge, Retry,
// the draft and read-only.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import { generateTab } from '../../shared/generateTabs';
import { projectStore } from '../../store/project';
import { regionEditFeature } from '../region-edit';
import { newAssetGenerationFeature } from '.';
import { gen } from './store';
import {
  genControls,
  mockGenCore,
  polyfillRadix,
  PROJECT_DIR,
  rgnControls,
  sketch,
  type GenSetup,
} from './testUtils';

vi.setConfig({ testTimeout: 15_000 });

const GeneratePanel = regionEditFeature.slots.right!;
const Tracker = regionEditFeature.slots.bottom!;
let fake: FakeTauri & { structure: unknown[] };

const tabIds = () => screen.queryAllByTestId('gen-tab').map((e) => e.dataset.recordId);
const selectedTab = () =>
  screen.queryAllByTestId('gen-tab').find((e) => e.getAttribute('aria-selected') === 'true')
    ?.dataset.recordId;
const controlIds = () =>
  screen.queryAllByTestId('gen-control-input').map((e) => e.dataset.recordId);
const control = (name: string) =>
  screen.getAllByTestId('gen-control-input').find((e) => e.dataset.recordId === name)!;

async function setup(opts: GenSetup & { tab?: 'new-image' | null } = {}) {
  polyfillRadix();
  expect(newAssetGenerationFeature.prefix).toBe('gen'); // registers the tab
  fake = mockGenCore(opts);
  projectStore.reset();
  if (opts.tab !== null) generateTab.openNextWith(opts.tab ?? 'new-image');
  await act(() => projectStore.open(PROJECT_DIR));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
      <Tracker />
    </Tooltip.Provider>,
  );
  if (opts.tab !== null) await waitFor(() => expect(gen.get().fit).not.toBeNull());
}

async function openSelect(trigger: HTMLElement): Promise<string[]> {
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: 'mouse' });
  const list = await screen.findByRole('listbox');
  return within(list)
    .getAllByRole('option')
    .map((o) => o.textContent ?? '');
}

async function choose(trigger: HTMLElement, label: string) {
  await openSelect(trigger);
  fireEvent.click(screen.getByRole('option', { name: label }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

async function pickModel(name: string) {
  await choose(screen.getByTestId('gen-panel-model'), name);
  await waitFor(() => expect(gen.get().fit).not.toBeNull());
}

function setInt(testId: string, value: number) {
  const el = screen.getByTestId(testId);
  fireEvent.change(el, { target: { value: String(value) } });
  fireEvent.keyDown(el, { key: 'Enter' });
}

async function estimated() {
  await waitFor(() => expect(gen.get().estimate.status).toBe('ready'), { timeout: 4000 });
}

async function generate(prompt = 'flooded crypt, torchlight') {
  fireEvent.change(screen.getByTestId('gen-panel-prompt'), { target: { value: prompt } });
  await estimated();
  await waitFor(() =>
    expect(screen.getByTestId<HTMLButtonElement>('gen-panel-submit').disabled).toBe(false),
  );
  await act(async () => fireEvent.click(screen.getByTestId('gen-panel-submit')));
  await screen.findByTestId('rgn-confirm');
  await act(async () => fireEvent.click(screen.getByTestId('rgn-confirm-ok')));
  await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
}

beforeEach(() => projectStore.reset());
afterEach(() => {
  act(() => projectStore.reset());
  cleanup();
  fake?.uninstall();
  generateTab.select('region');
});

test('FR-GEN-04 OQ-GEN-09 the tab strip is Region edit | New image; each tab mounts only while shown', async () => {
  await setup({ tab: null });
  await waitFor(() => expect(tabIds()).toEqual(['region', 'new-image']));
  expect(selectedTab()).toBe('region');
  expect(screen.getByTestId('prm-block-prompt')).toBeTruthy();
  expect(screen.queryByTestId('gen-panel')).toBeNull();

  fireEvent.click(screen.getAllByTestId('gen-tab')[1]!);
  expect(selectedTab()).toBe('new-image');
  await screen.findByTestId('gen-panel');
  expect(screen.queryByTestId('prm-block-prompt')).toBeNull();
  // Arrow keys move along the strip.
  fireEvent.keyDown(screen.getAllByTestId('gen-tab')[1]!, { key: 'ArrowLeft' });
  expect(selectedTab()).toBe('region');
  expect(screen.queryByTestId('gen-panel')).toBeNull();
});

test('FR-GEN-03 FR-GEN-04 a project with no image shows only the New image tab', async () => {
  await setup({ noImage: true, tab: null });
  await waitFor(() => expect(tabIds()).toEqual(['new-image']));
  expect(selectedTab()).toBe('new-image');
  expect(screen.getByTestId('gen-panel')).toBeTruthy();
  expect(screen.queryByTestId('prm-block-prompt')).toBeNull();
});

test('FR-GEN-01 a project opened from Generate new… starts on the New image tab', async () => {
  await setup();
  expect(selectedTab()).toBe('new-image');
});

test('FR-GEN-05 FR-GEN-06 FR-GEN-11 generate models and capabilities; output-size inputs are never controls', async () => {
  await setup();
  expect(fake.callsOf('provider_list_models').at(-1)).toEqual({ operation: 'generate' });
  expect(fake.callsOf('provider_get_capabilities').at(-1)).toEqual({
    model_id: 'fake-gen-wh',
    operation: 'generate',
  });
  await waitFor(() => expect(controlIds()).toEqual(['num_samples', 'background']));
  // The size is the Size block's, never a raw control.
  expect(controlIds()).not.toContain('width');
  expect(screen.getByTestId('gen-size').dataset.mode).toBe('grid');
  expect(screen.queryByTestId('gen-panel-count-single')).toBeNull();
  expect(fake.callsOf('gen_controls_evaluate').length).toBeGreaterThan(0);

  await pickModel('Fake gen (aspect)');
  await waitFor(() => expect(controlIds()).toEqual([]));
  expect(screen.getByTestId('gen-panel-count-single').textContent).toContain('1 result per job');
});

test('FR-GEN-07 FR-GEN-08 Maps: 30 × 40 at 70 by default; the model limit lowers px per square', async () => {
  await setup();
  expect(screen.getByTestId<HTMLInputElement>('gen-size-cols').value).toBe('30');
  expect(screen.getByTestId<HTMLInputElement>('gen-size-rows').value).toBe('40');
  expect(screen.getByTestId<HTMLInputElement>('gen-size-pps').value).toBe('70');
  expect(fake.callsOf('gen_size_fit').at(-1)).toEqual({
    model_id: 'fake-gen-wh',
    kind: 'map',
    size: { mode: 'grid', cols: 30, rows: 40, pps: 70 },
  });
  expect(screen.getByTestId('gen-size-result').textContent).toBe(
    'sent 1520 × 2032 · 50.7 × 50.8 px per square',
  );
  const notices = screen.getByTestId('gen-size-notices');
  expect(within(notices).getByText(/Model limit/).dataset.code).toBe('model_limit');

  setInt('gen-size-pps', 32);
  await waitFor(() =>
    expect(screen.getByTestId('gen-size-result').textContent).toBe(
      'sent 960 × 1280 · 32.0 × 32.0 px per square',
    ),
  );
  expect(screen.queryByTestId('gen-size-notices')).toBeNull();
  // Out of range is clamped before it is sent (cells 1–200, pps 16–512).
  setInt('gen-size-cols', 500);
  await waitFor(() => expect(gen.get().size).toMatchObject({ cols: 200 }));
});

test('FR-GEN-09 aspect models: the nearest ratio, with "cells won’t be square" when off by > 1 %', async () => {
  await setup();
  await pickModel('Fake gen (aspect)');
  setInt('gen-size-cols', 25);
  await waitFor(() =>
    expect(screen.getByTestId('gen-size-result').textContent).toBe('sent ratio 2:3'),
  );
  expect(screen.getByTestId('gen-size-notices').textContent).toContain('won’t be square');
  setInt('gen-size-cols', 30);
  setInt('gen-size-rows', 40);
  await waitFor(() =>
    expect(screen.getByTestId('gen-size-result').textContent).toBe('sent ratio 3:4'),
  );
  expect(screen.queryByTestId('gen-size-notices')).toBeNull();
});

test('FR-GEN-10 Tokens and Items: a ratio select (fixed list + long side on wh models, the model’s ratios on aspect models)', async () => {
  await setup({ kind: 'item' });
  expect(screen.queryByTestId('gen-size-cols')).toBeNull();
  expect(screen.getByTestId('gen-size-aspect').textContent).toContain('1:1');
  expect(await openSelect(screen.getByTestId('gen-size-aspect'))).toEqual([
    '1:1',
    '4:5',
    '5:4',
    '3:4',
    '4:3',
    '2:3',
    '3:2',
    '9:16',
    '16:9',
  ]);
  fireEvent.click(screen.getByRole('option', { name: '2:3' }));
  await waitFor(() =>
    expect(screen.getByTestId('gen-size-result').textContent).toBe('sent 672 × 1024'),
  );
  expect(screen.getByTestId<HTMLInputElement>('gen-size-long-side').value).toBe('1024');
  setInt('gen-size-long-side', 4000); // snapped into the model's 64–2048
  await waitFor(() => expect(gen.get().size).toMatchObject({ long_side: 2048 }));

  await pickModel('Fake gen (aspect)');
  expect(screen.queryByTestId('gen-size-long-side')).toBeNull();
  expect(await openSelect(screen.getByTestId('gen-size-aspect'))).toEqual([
    '1:1',
    '3:4',
    '4:3',
    '2:3',
  ]);
});

test('FR-GEN-12 OQ-GEN-11 Tokens default to a transparent background; Maps keep the model default', async () => {
  await setup({ kind: 'token' });
  await waitFor(() => expect(control('background').textContent).toContain('transparent'));
  expect(gen.get().touched).toContain('background'); // sent (FR-RGN-34)
  act(() => projectStore.reset());
  cleanup();
  fake.uninstall();
  await setup({ kind: 'map' });
  await waitFor(() => expect(control('background').textContent).toContain('opaque'));
});

test('FR-GEN-14 the sketch block: None by default, structure/ images by name, badges, Add…, hidden without a sketch input', async () => {
  await setup({
    sketches: [sketch('b-room.png'), sketch('a-room.png'), sketch('c-bad.png', 'corrupt')],
  });
  const block = await screen.findByTestId('gen-sketch');
  await waitFor(() =>
    expect(
      within(block)
        .getAllByTestId('gen-sketch-item')
        .map((e) => e.dataset.recordId),
    ).toEqual(['a-room.png', 'b-room.png', 'c-bad.png']),
  );
  expect(screen.getByTestId('gen-sketch-none').getAttribute('aria-checked')).toBe('true');
  const bad = within(block).getAllByTestId('gen-sketch-item')[2]!;
  expect(bad.hasAttribute('disabled')).toBe(true);
  expect(within(block).getByTestId('gen-sketch-badge').dataset.badge).toBe('corrupt');

  fireEvent.click(within(block).getAllByTestId('gen-sketch-item')[0]!);
  expect(gen.get().sketch).toBe('a-room.png');

  // No native dialog here: Add… falls back to typed paths, copied in through project_folder_add.
  await act(async () => fireEvent.click(screen.getByTestId('gen-sketch-add')));
  fireEvent.change(await screen.findByTestId('gen-sketch-add-paths'), {
    target: { value: 'D:/layouts/crypt.png' },
  });
  await act(async () => fireEvent.click(screen.getByTestId('gen-sketch-add-ok')));
  await waitFor(() => expect(within(block).getAllByTestId('gen-sketch-item')).toHaveLength(4));
  expect(fake.callsOf('project_folder_add').at(-1)).toEqual({
    folder: 'structure',
    paths: ['D:/layouts/crypt.png'],
  });

  await pickModel('Fake gen (aspect)');
  expect(screen.queryByTestId('gen-sketch')).toBeNull();
  expect(screen.getByTestId('gen-sketch-unsupported').textContent).toBe(
    'This model takes no structural sketch',
  );
});

test('FR-GEN-13 the References block resolves for generate; a chosen sketch takes one place in its slot', async () => {
  await setup({ sketches: [sketch('a-room.png')], projectRefs: ['look.png'] });
  await waitFor(() => expect(screen.getByTestId('ref-block-count').textContent).toBe('1 / 4'));
  expect(fake.callsOf('ref_resolve').at(-1)).toMatchObject({
    model_id: 'fake-gen-wh',
    operation: 'generate',
    structure: null,
  });
  fireEvent.click(await screen.findByTestId('gen-sketch-item'));
  await waitFor(() => expect(screen.getByTestId('ref-block-count').textContent).toBe('1 / 3'));
  expect(fake.callsOf('ref_resolve').at(-1)).toMatchObject({ structure: 'a-room.png' });
  fireEvent.click(screen.getByTestId('gen-sketch-none'));
  await waitFor(() => expect(screen.getByTestId('ref-block-count').textContent).toBe('1 / 4'));
});

test('FR-GEN-20 FR-GEN-21 FR-GEN-22 FR-GEN-25 estimate, confirm "New image · W × H", submit through RGN’s runner, tracker badge', async () => {
  await setup({ projectRefs: ['look.png'] });
  await estimated();
  const est = fake.callsOf('gen_job_estimate').length;
  // The prompt never re-estimates (FR-GEN-20).
  fireEvent.change(screen.getByTestId('gen-panel-prompt'), { target: { value: 'flooded crypt' } });
  await act(() => new Promise((r) => setTimeout(r, 700)));
  expect(fake.callsOf('gen_job_estimate').length).toBe(est);
  expect(screen.getByTestId('gen-panel-estimate').dataset.cost).toBe('2');

  await act(async () => fireEvent.click(screen.getByTestId('gen-panel-submit')));
  const dialog = await screen.findByTestId('rgn-confirm');
  expect(within(dialog).getByTestId('gen-confirm-summary').textContent).toBe(
    'New image · 1520 × 2032',
  );
  expect(within(dialog).getByTestId('rgn-confirm-references').textContent).toContain(
    'with 1 reference',
  );
  await act(async () => fireEvent.click(screen.getByTestId('rgn-confirm-ok')));
  await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());

  const sent = fake.callsOf('gen_job_submit').at(-1)!;
  expect(sent.draft).toMatchObject({
    model_id: 'fake-gen-wh',
    size: { mode: 'grid', cols: 30, rows: 40, pps: 70 },
    structure: null,
    typed_prompt: 'flooded crypt',
    final_prompt: 'flooded crypt',
    controls: {},
  });
  // One active job per project, shared with RGN (FR-GEN-22).
  await waitFor(() =>
    expect(screen.getByTestId('gen-panel-submit-reason').textContent).toBe('A job is running.'),
  );
  act(() => rgnControls().finish());
  const badge = await screen.findByTestId('gen-tracker-badge');
  expect(badge.textContent).toBe('new image · 1520 × 2032');
  expect(screen.getByTestId('rgn-tracker-item').dataset.n).toBe('1');
});

test('FR-GEN-32 Retry from the tracker fills the New image tab, flags a changed sketch, and estimates again', async () => {
  await setup({ sketches: [sketch('a-room.png')] });
  fireEvent.click(await screen.findByTestId('gen-sketch-item'));
  setInt('gen-size-cols', 20);
  await generate('crypt');
  act(() => rgnControls().finish());
  await screen.findByTestId('gen-tracker-badge');

  // Somewhere else meanwhile: Region edit, another prompt, the sketch edited in Explorer.
  fireEvent.change(screen.getByTestId('gen-panel-prompt'), { target: { value: 'other' } });
  fireEvent.click(screen.getByTestId('gen-sketch-none'));
  fireEvent.click(screen.getAllByTestId('gen-tab')[0]!);
  genControls().changeSketch('a-room.png');

  await act(async () => fireEvent.click(await screen.findByTestId('rgn-job-retry')));
  await waitFor(() => expect(selectedTab()).toBe('new-image'));
  await waitFor(() =>
    expect(screen.getByTestId<HTMLTextAreaElement>('gen-panel-prompt').value).toBe('crypt'),
  );
  expect(fake.callsOf('gen_job_retry_draft')).toHaveLength(1);
  expect(fake.callsOf('rgn_job_retry_draft')).toHaveLength(0);
  expect(gen.get().size).toMatchObject({ cols: 20 });
  expect(gen.get().sketch).toBe('a-room.png');
  expect((await screen.findByTestId('gen-sketch-flag')).textContent).toBe('changed since 001');
  expect(screen.getByTestId('gen-panel-info').textContent).toContain('filled from the job');
  await estimated();
});

test('FR-GEN-19 draft.generate is saved after an edit and restored on open', async () => {
  await setup({
    draft: {
      model_id: 'fake-gen-aspect',
      controls: {},
      seed: 7,
      size: { mode: 'grid', cols: 12, rows: 16, pps: 64 },
      structure: null,
      references: null,
      typed_prompt: 'swamp',
    },
  });
  expect(gen.get().modelId).toBe('fake-gen-aspect');
  expect(screen.getByTestId<HTMLInputElement>('gen-size-cols').value).toBe('12');
  expect(screen.getByTestId<HTMLTextAreaElement>('gen-panel-prompt').value).toBe('swamp');
  expect(screen.getByTestId<HTMLInputElement>('gen-panel-seed-value').value).toBe('7');

  setInt('gen-size-rows', 18);
  await waitFor(() => expect(fake.callsOf('project_put_draft').length).toBe(1), { timeout: 4000 });
  expect(fake.callsOf('project_put_draft')[0]).toEqual({
    draft: {
      generate: {
        model_id: 'fake-gen-aspect',
        controls: {},
        seed: 7,
        size: { mode: 'grid', cols: 12, rows: 18, pps: 64 },
        structure: null,
        references: null,
        typed_prompt: 'swamp',
      },
    },
  });
});

test('FR-GEN-34 read-only: the tab is shown, no estimate, Generate disabled with the reason', async () => {
  await setup({ readOnly: true });
  expect(screen.getByTestId('gen-panel-readonly')).toBeTruthy();
  expect(screen.getByTestId('gen-panel-estimate').dataset.status).toBe('idle');
  expect(screen.getByTestId<HTMLButtonElement>('gen-panel-submit').disabled).toBe(true);
  expect(screen.getByTestId('gen-panel-submit-reason').textContent).toBe(
    'The project is read-only.',
  );
  expect(fake.callsOf('gen_job_estimate')).toHaveLength(0);
});
