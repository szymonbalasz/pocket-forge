// GEN slice 4 on the mocked core (new-asset-generation FR-GEN-25–31): a generate iteration's
// candidates in RGN's tracker and EDT's picker are previewed whole (alone when the size differs
// or there is no image yet) and picked through `gen_candidate_*`, never RGN's bake; the pick is
// one undo step that reloads the project (back to "No image yet"), and redo brings it back.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Tooltip } from 'radix-ui';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';

import type { FakeTauri } from '../../api/mocks/tauri';
import { generateTab } from '../../shared/generateTabs';
import { projectStore } from '../../store/project';
import { editorFeature } from '../editor';
import { regionEditFeature } from '../region-edit';
import { newAssetGenerationFeature } from '.';
import { gen } from './store';
import { mockGenCore, polyfillRadix, PROJECT_DIR, rgnControls } from './testUtils';

vi.setConfig({ testTimeout: 15_000 });

const GeneratePanel = regionEditFeature.slots.right!;
const Tracker = regionEditFeature.slots.bottom!;
const Canvas = editorFeature.slots.canvas!;
const Picker = editorFeature.slots.bottom!;
let fake: FakeTauri;

async function setup(fromHome = false) {
  polyfillRadix();
  expect(newAssetGenerationFeature.prefix).toBe('gen');
  fake = mockGenCore({ noImage: true });
  projectStore.reset();
  // From home (not + New → Generate new…) the remembered tab is Region edit.
  if (fromHome) generateTab.select('region');
  else generateTab.openNextWith('new-image');
  await act(() => projectStore.open(PROJECT_DIR));
  render(
    <Tooltip.Provider>
      <Canvas />
      <GeneratePanel />
      <Tracker />
      <Picker />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(gen.get().fit).not.toBeNull());
}

async function generate(prompt: string, count = 2) {
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: prompt } });
  await waitFor(() => expect(gen.get().estimate.status).toBe('ready'), { timeout: 4000 });
  await act(async () => fireEvent.click(screen.getByTestId('gen-panel-submit')));
  await screen.findByTestId('rgn-confirm');
  await act(async () => fireEvent.click(screen.getByTestId('rgn-confirm-ok')));
  await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
  act(() => rgnControls().finish(count));
  await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(count));
}

const key = (k: string, extra: Partial<KeyboardEventInit> = {}) =>
  act(async () => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, ...extra }));
  });

beforeEach(() => projectStore.reset());
afterEach(() => {
  act(() => projectStore.reset());
  cleanup();
});

test('FR-GEN-26 FR-GEN-27 FR-GEN-29 preview alone with no image, Enter picks, undo returns to "No image yet", redo brings it back', async () => {
  await setup();
  await generate('flooded crypt');
  const items = screen.getAllByTestId('edt-picker-item');
  await act(async () => fireEvent.click(items[1]!));

  // FR-GEN-26: GEN's whole-image preview, shown alone (no working image to compare with).
  await waitFor(() => expect(fake.callsOf('gen_candidate_preview')).toHaveLength(1));
  expect(fake.callsOf('rgn_candidate_preview')).toHaveLength(0);
  const alone = await screen.findByTestId('edt-preview-alone');
  expect(alone.querySelector('img')!.getAttribute('src')).toContain('gp-');
  expect(screen.getByTestId('edt-canvas-no-image')).toBeTruthy();

  // FR-GEN-27: Enter = Pick, through GEN, never RGN's accept.
  await key('Enter');
  await waitFor(() => expect(fake.callsOf('gen_candidate_pick')).toHaveLength(1));
  expect(fake.callsOf('rgn_candidate_accept')).toHaveLength(0);
  await waitFor(() =>
    expect(projectStore.get().project?.image_path).toBe('working/001 flooded crypt 02.png'),
  );
  expect(screen.queryByTestId('edt-canvas-no-image')).toBeNull();
  await screen.findByText('New image picked.');

  // FR-GEN-29: one undo step restores the parent revision and reloads PRJ's snapshot.
  await key('z', { ctrlKey: true });
  await waitFor(() => expect(fake.callsOf('project_restore_revision')).toHaveLength(1));
  expect(fake.callsOf('project_restore_revision')[0]).toEqual({ rev_id: 'r0' });
  await waitFor(() => expect(projectStore.get().project?.image_path).toBeNull());
  await screen.findByTestId('edt-canvas-no-image');

  // Redo works from the "No image yet" state too (the panel owns the undo stack). A key while
  // the undo still runs is ignored, so it is pressed until the redo goes through.
  await waitFor(async () => {
    await key('y', { ctrlKey: true });
    expect(fake.callsOf('project_restore_revision')).toHaveLength(2);
  });
  await waitFor(() =>
    expect(projectStore.get().project?.image_path).toBe('working/001 flooded crypt 02.png'),
  );
});

test('FR-GEN-30 FR-GEN-31 any generate iteration can be picked; discard goes through RGN', async () => {
  await setup();
  await generate('crypt');
  await act(async () => fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!));
  await key('Enter');
  await waitFor(() => expect(projectStore.get().project?.has_image).toBe(true));

  await generate('second crypt');
  // Back to the older 001: its candidates are shown and #2 can be picked (no newest-only rule).
  const chips = screen.getAllByTestId('rgn-tracker-item');
  const older = chips.find((c) => c.dataset.n === '1')!;
  await act(async () => fireEvent.click(older));
  const olderJob = older.dataset.recordId!;
  await waitFor(() =>
    expect(screen.getAllByTestId('edt-picker-item')[0]!.dataset.recordId).toContain(olderJob),
  );
  const items = screen.getAllByTestId('edt-picker-item');
  await act(async () => fireEvent.click(items[1]!));
  await waitFor(() => expect(fake.callsOf('gen_candidate_preview').length).toBeGreaterThan(0));
  await key('Enter');
  await waitFor(() => expect(fake.callsOf('gen_candidate_pick')).toHaveLength(2));
  expect(fake.callsOf('gen_candidate_pick')[1]).toMatchObject({
    job_id: older.dataset.recordId,
  });
  await waitFor(() =>
    expect(projectStore.get().project?.image_path).toBe('working/001 crypt 02.png'),
  );
});

test('R84 after a pick the panel stays on the New image tab (also when opened from home)', async () => {
  await setup(true);
  expect(
    screen.getAllByTestId('gen-tab').find((t) => t.getAttribute('aria-selected') === 'true')
      ?.dataset.recordId,
  ).toBe('new-image');
  await generate('crypt');
  await act(async () => fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!));
  await key('Enter');
  await waitFor(() => expect(projectStore.get().project?.has_image).toBe(true));
  await waitFor(() => expect(screen.getAllByTestId('gen-tab')).toHaveLength(2));
  expect(
    screen.getAllByTestId('gen-tab').find((t) => t.getAttribute('aria-selected') === 'true')
      ?.dataset.recordId,
  ).toBe('new-image');
  expect(screen.getByTestId('gen-panel')).toBeTruthy();
});

test('FR-GEN-32 walk-p4r L6 the Retry note "filled from the job" ends with the next pick', async () => {
  await setup();
  await generate('crypt');
  await act(async () => fireEvent.click(await screen.findByTestId('rgn-job-retry')));
  await act(async () => fireEvent.click(await screen.findByTestId('rgn-job-retry-ask-confirm')));
  await waitFor(() =>
    expect(screen.getByTestId('gen-panel-info').textContent).toContain('filled from the job'),
  );
  await act(async () => fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!));
  await key('Enter');
  await waitFor(() => expect(projectStore.get().project?.has_image).toBe(true));
  await waitFor(() => expect(screen.queryByTestId('gen-panel-info')).toBeNull());
});

test('FR-GEN-29 walk-p4r L6 a refused pick notice ends with the next submit', async () => {
  await setup();
  await generate('crypt');
  const w = window as unknown as { __pfMockHandlers: Record<string, (i: never) => unknown> };
  const pick = w.__pfMockHandlers.gen_candidate_pick!;
  w.__pfMockHandlers.gen_candidate_pick = () =>
    Promise.reject({ code: 'job_active', message: 'a job is already running' });
  await act(async () => fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!));
  await key('Enter');
  await waitFor(() =>
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toContain(
      'A job is already running in this project.',
    ),
  );
  w.__pfMockHandlers.gen_candidate_pick = pick;
  await generate('crypt again', 3);
  expect(screen.queryByTestId('rgn-tracker-notice')).toBeNull();
});

const press = (k: string) =>
  window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));

test('FR-GEN-29 R97 two quick picks run in turn: undo goes back to the first, and Enter twice makes one revision', async () => {
  await setup();
  await generate('crypt', 3);
  const image = () => projectStore.get().project?.image_path;
  // #2 then #3 inside one round trip.
  await act(async () => {
    press('2');
    press('Enter');
    press('3');
    press('Enter');
  });
  await waitFor(() => expect(fake.callsOf('gen_candidate_pick')).toHaveLength(2));
  await waitFor(() => expect(image()).toBe('working/001 crypt 03.png'));

  // Enter twice on #1: one pick, one revision.
  await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3));
  await act(async () => {
    press('1');
    press('Enter');
    press('Enter');
  });
  await waitFor(() => expect(image()).toBe('working/001 crypt 01.png'));
  await act(() => new Promise((r) => setTimeout(r, 50)));
  expect(fake.callsOf('gen_candidate_pick')).toHaveLength(3);

  // Undo walks back one pick at a time: #3, then #2 (never past it to "No image yet").
  await key('z', { ctrlKey: true });
  await waitFor(() => expect(image()).toBe('working/001 crypt 03.png'));
  await waitFor(async () => {
    await key('z', { ctrlKey: true });
    expect(fake.callsOf('project_restore_revision')).toHaveLength(2);
  });
  await waitFor(() => expect(image()).toBe('working/001 crypt 02.png'));
  expect(fake.callsOf('project_restore_revision').map((c) => c.rev_id)).not.toContain('r0');
});
