// Walk 2 N1: after a sync conflict is resolved the Generate panel shows the kept version's draft,
// and nothing the user typed before writes the version not kept back (FR-PRJ-36).
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import { api } from '../../api';
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

type Win = {
  __pfMockHandlers: Record<string, (i: unknown) => unknown>;
  __pfMockSnapshot: Record<string, unknown>;
};

async function openWithConflictOnSave(keptPrompt: string): Promise<void> {
  polyfillRadix();
  fake = mockCore({
    failures: { project_put_draft: { code: 'conflict', message: 'also changed elsewhere' } },
  });
  const w = window as unknown as Win;
  // Keep other: the kept version's draft becomes current; this machine's is a revision.
  w.__pfMockHandlers.project_resolve_conflict = () => {
    w.__pfMockSnapshot.draft = { final_prompt: keptPrompt, seed: 7, model_id: null };
    return { id: 'r9', seq: 9, created_at: '2026-11-07T11:00:00.000Z', reason: 'conflict' };
  };
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
}

test('FR-PRJ-36 Keep other: the panel shows the kept prompt and a seed toggle saves it, not ours (walk 2 N1)', async () => {
  await openWithConflictOnSave('theirs');
  fireEvent.change(screen.getByTestId('rgn-panel-prompt'), { target: { value: 'ours, unsaved' } });
  await waitFor(() => expect(projectStore.get().save).toMatchObject({ code: 'conflict' }), {
    timeout: DRAFT_SAVE_MS + 2000,
  });

  await act(async () => {
    await api.project.resolveConflict('other');
    await projectStore.resolved();
  });
  await waitFor(() =>
    expect((screen.getByTestId('rgn-panel-prompt') as HTMLTextAreaElement).value).toBe('theirs'),
  );
  expect(rgn.get().seed).toBe(7);
  expect(projectStore.get().save.kind).toBe('saved');

  const before = fake.callsOf('project_put_draft').length;
  act(() => rgn.setSeedMode('random'));
  await waitFor(() => expect(fake.callsOf('project_put_draft').length).toBe(before + 1), {
    timeout: DRAFT_SAVE_MS + 2000,
  });
  const sent = fake.callsOf('project_put_draft').at(-1) as { draft: { final_prompt: string } };
  expect(sent.draft.final_prompt).toBe('theirs');
}, 15_000);

test('FR-PRJ-36 a draft save still waiting at the resolve is dropped, not written over the kept version', async () => {
  await openWithConflictOnSave('theirs');
  fireEvent.change(screen.getByTestId('rgn-panel-prompt'), { target: { value: 'typed just now' } });
  // Resolved before the debounced save fires.
  await act(async () => {
    await api.project.resolveConflict('other');
    await projectStore.resolved();
  });
  await new Promise((r) => setTimeout(r, DRAFT_SAVE_MS + 200));
  expect(fake.callsOf('project_put_draft')).toHaveLength(0);
  expect((screen.getByTestId('rgn-panel-prompt') as HTMLTextAreaElement).value).toBe('theirs');
}, 15_000);
