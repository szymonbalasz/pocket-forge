// PRJ's close guards (R51, R55): a feature holds the close (EDT's last mask submit, EXP's prompt)
// and lets it go on; `close` resolves once it really closed, so callers can check the outcome.
import { afterEach, beforeEach, expect, test } from 'vitest';

import { prjMock, samplePrjSeed } from '../api/mocks/prj';
import { installTestCore } from '../api/mocks/testCore';
import { projectStore, type CloseGuard } from './project';

const core = installTestCore();
const LIB = 'C:/pf-mock/library';
const CRYPT = `${LIB}/Maps/Crypt`;
const offs: (() => void)[] = [];

const guard = (g: CloseGuard) => offs.push(projectStore.addCloseGuard(g));
const cmds = () => core.calls.map((c) => c.cmd);

beforeEach(async () => {
  core.useHandlers(prjMock(samplePrjSeed(LIB), { emit: core.emit }).handlers);
  projectStore.reset();
  await projectStore.open(CRYPT);
  core.calls.length = 0;
});

afterEach(() => {
  offs.splice(0).forEach((f) => f());
  projectStore.reset();
});

test('FR-EDT-28 R55 a held close waits for the guard, then closes; close resolves after it', async () => {
  let release!: () => void;
  guard((proceed) => {
    release = proceed;
    return false;
  });
  let done = false;
  const closing = projectStore.close().then(() => (done = true));
  await Promise.resolve();
  expect(done).toBe(false);
  expect(cmds()).not.toContain('project_close');
  release();
  await closing;
  expect(cmds()).toContain('project_close');
  expect(projectStore.get().project).toBeNull();
});

test('R55 proceeding from a held guard still asks the guards after it', async () => {
  const asked: string[] = [];
  guard((proceed) => {
    asked.push('mask');
    queueMicrotask(proceed);
    return false;
  });
  guard((_proceed, cancel) => {
    asked.push('export');
    cancel(); // "Cancel the export and close?" → no
    return false;
  });
  await projectStore.close();
  expect(asked).toEqual(['mask', 'export']);
  expect(cmds()).not.toContain('project_close');
  expect(projectStore.get().project).not.toBeNull();
});

test('FR-PRJ-14 R55 opening another project asks the guards before closing this one', async () => {
  let held = 0;
  guard((proceed) => {
    held++;
    queueMicrotask(proceed);
    return false;
  });
  expect(await projectStore.open(`${LIB}/Maps/Harbour`)).toBe(true);
  expect(held).toBe(1);
  expect(projectStore.get().project?.name).toBe('Harbour');
  expect(cmds().indexOf('project_close')).toBeLessThan(cmds().indexOf('project_open'));
});
