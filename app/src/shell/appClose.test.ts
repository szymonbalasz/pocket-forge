// R101: the window X is a project close: the same guards, then exit; a refused close stays open.
import { afterEach, beforeEach, expect, test } from 'vitest';

import { prjMock, samplePrjSeed, type PrjMockProject } from '../api/mocks/prj';
import { installTestCore } from '../api/mocks/testCore';
import { projectStore } from '../store/project';
import { installAppClose } from './appClose';

const core = installTestCore();
const LIB = 'C:/pf-mock/library';
const CRYPT = `${LIB}/Maps/Crypt`;
const offs: (() => void)[] = [];

const names = () => core.names();
const flush = () => new Promise((r) => setTimeout(r, 0));

async function start(patch: Partial<PrjMockProject> | null): Promise<void> {
  const seed = samplePrjSeed(LIB);
  seed.projects[0] = { ...seed.projects[0]!, ...patch };
  core.useHandlers({
    ...prjMock(seed, { emit: core.emit }).handlers,
    app_close_guard: () => null,
    app_exit: () => null,
  });
  projectStore.reset();
  if (patch) await projectStore.open(CRYPT);
  offs.push(installAppClose());
  await flush();
  core.calls.length = 0;
}

async function pressX(): Promise<void> {
  core.emit('app:close-requested', null);
  await flush();
}

beforeEach(() => projectStore.reset());
afterEach(() => {
  offs.splice(0).forEach((f) => f());
  projectStore.reset();
});

test('R101 the app tells the core its close guards listen', async () => {
  core.useHandlers({ app_close_guard: () => null });
  offs.push(installAppClose());
  await flush();
  expect(names()).toContain('app_close_guard');
});

test('R101 the window X with no project open exits', async () => {
  await start(null);
  await pressX();
  expect(names()).toEqual(['app_exit']);
});

test('R101 FR-PRJ-21 the window X closes the project first, then exits', async () => {
  await start({});
  await pressX();
  expect(names()).toEqual(['project_close', 'app_exit']);
  expect(projectStore.get().project).toBeNull();
});

test('R101 FR-PRJ-21 the window X with a waiting save asks first; Close anyway then exits', async () => {
  await start({ pendingSave: true });
  await pressX();
  expect(names()).toEqual(['project_close']);
  expect(projectStore.get().error?.code).toBe('unsaved');
  expect(projectStore.get().project).not.toBeNull();
  await projectStore.close(true);
  await flush();
  expect(names()).toEqual(['project_close', 'project_close', 'app_exit']);
});

test('R101 FR-PRJ-21 Dismiss drops the quit: a later close only closes', async () => {
  await start({ pendingSave: true });
  await pressX();
  projectStore.clearError();
  await projectStore.close(true);
  await flush();
  expect(names()).not.toContain('app_exit');
});

test('R101 OQ-EXP-09 FR-EDT-28 a held guard holds the exit; cancelling keeps the app open', async () => {
  await start({});
  let answer!: { proceed: () => void; cancel: () => void };
  offs.push(
    projectStore.addCloseGuard((proceed, cancel) => {
      answer = { proceed, cancel };
      return false;
    }),
  );
  await pressX();
  await pressX(); // a second X while the question is open asks nothing more
  expect(names()).toEqual([]);
  answer.cancel();
  await flush();
  expect(names()).toEqual([]);
  expect(projectStore.get().project).not.toBeNull();
  // Asked again and allowed: the close goes on, then the exit.
  await pressX();
  answer.proceed();
  await flush();
  expect(names()).toEqual(['project_close', 'app_exit']);
});
