// Walk D7: the open-time "N job(s) can resume" banner ends with the last running job, and the
// session spend uses the provider's unit (the fake's "fake"), never a hard-coded "CU".
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { GeneratePanel } from './GeneratePanel';
import { rgn } from './store';
import { mockControls, mockCore, polyfillRadix } from './testUtils';

type W = {
  __pfMockSnapshot: Record<string, unknown>;
  __pfMockHandlers: Record<string, (i: unknown) => unknown>;
};
const w = () => window as unknown as W;
const resumable = () => projectStore.get().project?.warnings.some((x) => x.code === 'resumable');

let fake: FakeTauri;

beforeEach(() => {
  rgn.detach();
});
afterEach(() => {
  cleanup();
  rgn.detach();
  fake.uninstall();
});

async function open(): Promise<void> {
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
}

test('FR-RGN-22 walk D7 the "can resume" banner stays while the job runs and ends with it', async () => {
  polyfillRadix();
  fake = mockCore();
  w().__pfMockSnapshot.warnings = [{ code: 'resumable', message: '1 job(s) can resume.' }];
  // A job left running by the last session (FR-RGN-22 resumes it on open).
  (mockControls().state() as unknown as { jobs: unknown[] }).jobs.push({
    id: '01JJOBRESUME',
    seq: 1,
    state: 'running',
    draft: {
      region: { inner: { x: 400, y: 400, w: 512, h: 512 }, margin: 128 },
      controls: {},
      final_prompt: 'a well',
      model_id: 'fake-all-types',
    },
    outer: { x: 272, y: 272, w: 768, h: 768 },
    cost: 2,
    n: null,
    dir: null,
    slug: null,
    resolved: false,
    error: null,
    progress: 0.4,
    refs: [],
  });
  await open();
  await waitFor(() => expect(rgn.get().iterations).toHaveLength(1));
  expect(resumable()).toBe(true);
  act(() => mockControls().finish(2));
  await waitFor(() => expect(resumable()).toBe(false));
});

test('walk D7 the session spend uses the provider’s unit before any estimate (fake: "fake")', async () => {
  polyfillRadix();
  fake = mockCore({ unit: 'fake' });
  const status = w().__pfMockHandlers.provider_status!;
  w().__pfMockHandlers.provider_status = (i) => ({ ...(status(i) as object), mode: 'fake' });
  await open();
  await waitFor(() => expect(screen.getByTestId('rgn-panel-spend').textContent).toContain('fake'));
  expect(screen.getByTestId('rgn-panel-spend').textContent).toBe('Session 0 fake of 100 fake cap');
});
