// Walk D3 (RGN part): a failed draft write must not look saved (FR-PRJ-21).
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
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
  vi.useRealTimers();
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

  // BL-28: drive the debounce with fake timers instead of waiting two real 2 s debounces.
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: 'flooded crypt' } });
  await act(() => vi.advanceTimersByTimeAsync(DRAFT_SAVE_MS - 1));
  expect(fake.callsOf('project_put_draft')).toHaveLength(0);
  await act(() => vi.advanceTimersByTimeAsync(1));
  expect(fake.callsOf('project_put_draft')).toHaveLength(1);
  expect(projectStore.get().save).toMatchObject({ kind: 'failed', code: 'not_found' });

  // The next change saves again (the mock fails once); its commit sets the segment back.
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: 'flooded hall' } });
  await act(() => vi.advanceTimersByTimeAsync(DRAFT_SAVE_MS));
  expect(fake.callsOf('project_put_draft')).toHaveLength(2);
  act(() => fake.emit('project:committed', { seq: 2 }));
  expect(projectStore.get().save.kind).toBe('saved');
});

const mockWindow = () =>
  window as unknown as {
    __pfMockHandlers: Record<string, (i: Record<string, unknown>) => unknown>;
    __pfMockSnapshot: Record<string, unknown>;
  };

async function openPanel(): Promise<void> {
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
}

test('FR-PRJ-19 R97 a region edit still waiting for its save is written on close and on project switch', async () => {
  await openPanel();
  const order = () =>
    fake.calls.map((c) => c.cmd).filter((c) => c === 'project_put_draft' || c === 'project_close');
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: 'moss' } });
  await act(() => projectStore.close());
  expect(order()).toEqual(['project_put_draft', 'project_close']);
  expect(fake.callsOf('project_put_draft')[0]).toMatchObject({ draft: { final_prompt: 'moss' } });

  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: 'ivy' } });
  mockWindow().__pfMockSnapshot.project_id = '01JPROJECTB000000000000000';
  await act(() => projectStore.open('C:/pf-test/library/Maps/other'));
  expect(order()).toEqual([
    'project_put_draft',
    'project_close',
    'project_put_draft',
    'project_close',
  ]);
  expect(fake.callsOf('project_put_draft')[1]).toMatchObject({ draft: { final_prompt: 'ivy' } });
});

test('FR-GEN-19 R97 a late Retry answer is dropped when another project opened meanwhile', async () => {
  await openPanel();
  let release: () => void = () => undefined;
  mockWindow().__pfMockHandlers.rgn_job_retry_draft = () =>
    new Promise<void>((r) => (release = r)).then(() => ({
      final_prompt: 'from project A',
      seed: 7,
      blend_instruction: true,
      prompt_helpers: null,
      region: { inner: { x: 0, y: 0, w: 256, h: 256 }, margin: 32 },
      mask_id: null,
      model_id: 'fake-all-types',
      controls: {},
      references: null,
    }));
  const retry = rgn.retry('01JJOBA0000000000000000000');
  mockWindow().__pfMockSnapshot.project_id = '01JPROJECTB000000000000000';
  await act(() => projectStore.open('C:/pf-test/library/Maps/other'));
  await waitFor(() => expect(rgn.get().projectId).toBe('01JPROJECTB000000000000000'));
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  const saves = fake.callsOf('project_put_draft').length;
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  await act(async () => {
    release();
    await retry;
  });
  await act(() => vi.advanceTimersByTimeAsync(DRAFT_SAVE_MS + 10));
  expect(rgn.get().prompt).not.toBe('from project A');
  expect(rgn.get().seed).not.toBe(7);
  expect(fake.callsOf('project_put_draft').slice(saves)).toEqual([]);
});

test('R109 FR-PRJ-19 a second close while the close-time save runs waits for it', async () => {
  await openPanel();
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  mockWindow().__pfMockHandlers.project_put_draft = async () => {
    await gate;
    return { seq: 9 };
  };
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: 'moss' } });
  let first!: Promise<void>;
  let second!: Promise<void>;
  act(() => {
    // The window X, then ← Home at once.
    first = projectStore.close();
    second = projectStore.close();
  });
  await act(() => new Promise((r) => setTimeout(r, 0)));
  expect(fake.callsOf('project_put_draft')).toHaveLength(1);
  expect(fake.callsOf('project_close'), 'no close before the save lands').toHaveLength(0);
  release();
  await act(() => Promise.all([first, second]));
  expect(fake.callsOf('project_close').length).toBeGreaterThan(0);
  expect(projectStore.get().project).toBeNull();
});
