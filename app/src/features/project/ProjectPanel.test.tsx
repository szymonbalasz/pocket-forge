// PRJ left panel and status segment against the mocked PRJ commands (api/mocks/prj.ts).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';

import {
  prjMock,
  samplePrjSeed,
  type PrjMockProject,
  type PrjMockState,
} from '../../api/mocks/prj';
import { installTestCore } from '../../api/mocks/testCore';
import { projectStore } from '../../store/project';
import { ProjectPanel } from './ProjectPanel';
import { ProjectStatus } from './ProjectStatus';

const core = installTestCore();
const LIB = 'C:/pf-mock/library';
const CRYPT = `${LIB}/Maps/Crypt`;
let mock: PrjMockState;

async function openCrypt(patch: Partial<PrjMockProject> = {}) {
  const seed = samplePrjSeed(LIB);
  seed.projects[0] = { ...seed.projects[0]!, ...patch };
  const m = prjMock(seed, { emit: core.emit });
  core.useHandlers(m.handlers);
  mock = m.state;
  render(
    <>
      <ProjectPanel />
      <footer>
        <ProjectStatus />
      </footer>
    </>,
  );
  await act(() => projectStore.open(CRYPT));
  await screen.findByTestId('prj-panel');
}

const files = (folder: string) =>
  screen.queryAllByTestId(`prj-panel-${folder}-file`).map((f) => f.dataset.recordId);

beforeEach(() => projectStore.reset());
afterEach(cleanup);

test('FR-PRJ-48 FR-PRJ-12 the panel lists Working and the user folders on open', async () => {
  await openCrypt({
    folders: {
      references: [
        { name: 'mood.jpg', bytes: 2048, mtime: '2026-11-01T00:00:00Z', image: true },
        { name: 'notes.txt', bytes: 12, mtime: '2026-11-01T00:00:00Z', image: false },
      ],
    },
  });
  await waitFor(() => expect(files('working')).toEqual(['Crypt.png']));
  await waitFor(() => expect(files('references')).toEqual(['mood.jpg', 'notes.txt']));
  const [img, txt] = screen.getAllByTestId('prj-panel-references-file');
  expect(img!.dataset.image).toBe('true');
  expect(txt!.dataset.image).toBeUndefined();
  expect(files('structure')).toEqual([]);
  expect(files('parts')).toEqual([]);
  for (const id of ['results', 'history', 'storage'])
    expect(screen.getByTestId(`prj-panel-${id}`)).toBeTruthy();
  expect(screen.getByTestId('prj-panel-name').textContent).toBe('Crypt');
  expect(screen.getByTestId('prj-panel').dataset.kind).toBe('map');
});

test('FR-PRJ-48 files dropped into a user folder in Explorer appear on Refresh and on window focus', async () => {
  await openCrypt();
  await waitFor(() => expect(files('working')).toEqual(['Crypt.png']));
  mock.addUserFile(CRYPT, 'references', {
    name: 'statue.png',
    bytes: 900,
    mtime: '2026-11-08T00:00:00Z',
    image: true,
  });
  expect(files('references')).toEqual([]);
  fireEvent.click(screen.getByTestId('prj-panel-references-refresh'));
  await waitFor(() => expect(files('references')).toEqual(['statue.png']));

  mock.addUserFile(CRYPT, 'parts', {
    name: 'door.png',
    bytes: 900,
    mtime: '2026-11-08T00:00:00Z',
    image: true,
  });
  act(() => void window.dispatchEvent(new Event('focus')));
  await waitFor(() => expect(files('parts')).toEqual(['door.png']));
});

test('FR-PRJ-47 Add… copies files into a user-owned folder; Working (app-owned) has no Add', async () => {
  await openCrypt();
  expect(screen.queryByTestId('prj-panel-working-add')).toBeNull();
  fireEvent.click(screen.getByTestId('prj-panel-structure-add'));
  fireEvent.change(await screen.findByTestId('prj-panel-structure-add-paths'), {
    target: { value: '"D:/ref/walls.png"\nD:/ref/plan.pdf' },
  });
  fireEvent.click(screen.getByTestId('prj-panel-structure-add-confirm'));
  await waitFor(() => expect(files('structure')).toEqual(['walls.png', 'plan.pdf']));
  expect(core.calls.find((c) => c.cmd === 'project_folder_add')?.input).toEqual({
    folder: 'structure',
    paths: ['D:/ref/walls.png', 'D:/ref/plan.pdf'],
  });
});

test('FR-PRJ-23 History lists revisions newest first; Save with a label and Restore add revisions', async () => {
  await openCrypt();
  const items = () => screen.getAllByTestId('prj-panel-history-item');
  await waitFor(() => expect(items()).toHaveLength(1));
  expect(items()[0]!.dataset.reason).toBe('create');
  expect(items()[0]!.textContent).toContain('r0');

  fireEvent.change(screen.getByTestId('prj-panel-history-label'), {
    target: { value: 'before the river' },
  });
  fireEvent.click(screen.getByTestId('prj-panel-history-save'));
  await waitFor(() => expect(items()).toHaveLength(2));
  expect(items()[0]!.textContent).toContain('before the river');
  expect(screen.getByTestId('prj-status-save').dataset.state).toBe('saved');

  const r0 = items()[1]!;
  fireEvent.click(within(r0).getByTestId('prj-panel-history-restore'));
  await waitFor(() => expect(items()).toHaveLength(3));
  expect(items()[0]!.dataset.reason).toBe('restore');
  expect(core.calls.find((c) => c.cmd === 'project_restore_revision')?.input).toEqual({
    rev_id: r0.dataset.recordId,
  });
});

test('FR-PRJ-23 Ctrl+S with nothing changed reports no changes', async () => {
  await openCrypt();
  fireEvent.keyDown(window, { key: 's', ctrlKey: true });
  await waitFor(() =>
    expect(screen.getByTestId('prj-status-save').dataset.state).toBe('unchanged'),
  );
  expect(core.names()).toContain('project_save_revision');
});

test('FR-PRJ-33 FR-PRJ-31 FR-PRJ-32 Storage shows disk use, trash count, next purge, and restores a trashed item', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'results/001 kitchen/02.png',
        kind: 'candidate',
        bytes: 3 * 1024 * 1024,
        trashed_at: '2026-11-01T00:00:00Z',
      },
    ],
  });
  const trash = await screen.findByTestId('prj-panel-storage-trash');
  expect(trash.textContent).toContain('Trash: 1 item, 3.0 MB');
  expect(trash.textContent).toContain(
    `next purge ${new Date('2026-12-01T00:00:00Z').toLocaleDateString()}`,
  );
  expect(screen.getByTestId('prj-panel-storage-total').textContent).toBe('3.0 MB');
  fireEvent.click(screen.getByTestId('prj-panel-storage-restore'));
  await waitFor(() => expect(screen.queryByTestId('prj-panel-storage-item')).toBeNull());
  expect(core.calls.find((c) => c.cmd === 'project_trash_restore')?.input).toEqual({
    item_id: 'T1',
  });
});

test('FR-PRJ-21 closing with a save still pending asks first; Close anyway closes', async () => {
  await openCrypt({ pendingSave: true });
  fireEvent.click(screen.getByTestId('prj-panel-close'));
  const banner = await screen.findByTestId('prj-error');
  expect(banner.dataset.code).toBe('unsaved');
  expect(projectStore.get().project).not.toBeNull();
  fireEvent.click(screen.getByTestId('prj-panel-close-anyway'));
  await waitFor(() => expect(projectStore.get().project).toBeNull());
  expect(core.calls.filter((c) => c.cmd === 'project_close').map((c) => c.input)).toEqual([
    {},
    { force: true },
  ]);
});

test('FR-PRJ-41 FR-PRJ-15 a read-only project shows why and disables Add, Save and Restore', async () => {
  await openCrypt({ read_only: 'original_changed' });
  const w = await screen.findByTestId('prj-panel-warning');
  expect(w.dataset.code).toBe('original_changed');
  expect(screen.queryByTestId('prj-panel-references-add')).toBeNull();
  expect(screen.getByTestId<HTMLButtonElement>('prj-panel-history-save').disabled).toBe(true);
  await waitFor(() => expect(screen.getAllByTestId('prj-panel-history-item')).toHaveLength(1));
  expect(screen.getByTestId<HTMLButtonElement>('prj-panel-history-restore').disabled).toBe(true);
  expect(screen.getByTestId('prj-status-save').dataset.state).toBe('read_only');
  expect(screen.getByTestId('prj-status-save').textContent).toContain(
    'Read-only (original changed)',
  );
});

test('FR-PRJ-16 a missing original offers Relink, which accepts only the matching file', async () => {
  await openCrypt({
    read_only: 'original_missing',
    warnings: [{ code: 'original_missing', message: 'Original missing' }],
  });
  const path = await screen.findByTestId('prj-panel-relink-path');
  fireEvent.change(path, { target: { value: 'D:/backup/other.png' } });
  fireEvent.click(screen.getByTestId('prj-panel-relink'));
  expect((await screen.findByTestId('prj-panel-relink-error')).textContent).toBe(
    'That file is not the original.',
  );
  fireEvent.change(path, { target: { value: 'D:/backup/Crypt.png' } });
  fireEvent.click(screen.getByTestId('prj-panel-relink'));
  await waitFor(() => expect(screen.queryByTestId('prj-panel-warning')).toBeNull());
  expect(projectStore.get().project?.read_only).toBeNull();
});

test('FR-PRJ-21 FR-PRJ-35 save-failed and conflict events drive the status segment', async () => {
  await openCrypt();
  const seg = screen.getByTestId('prj-status-save');
  act(() => core.emit('project:save-failed', { reason: 'locked', retrying: true }));
  expect(seg.dataset.state).toBe('failed');
  expect(seg.textContent).toContain('Save failed, file locked, retrying');
  act(() => core.emit('project:committed', { seq: 9 }));
  expect(seg.dataset.state).toBe('saved');
  act(() => core.emit('project:conflict', {}));
  expect(seg.dataset.state).toBe('conflict');
  expect(seg.textContent).toContain('Conflict');
});

test('FR-PRJ-14 ← Home closes the project; the panel then shows the home hint', async () => {
  await openCrypt();
  fireEvent.click(screen.getByTestId('prj-panel-close'));
  await waitFor(() => expect(screen.getByTestId('prj-panel-home')).toBeTruthy());
  expect(screen.queryByTestId('prj-status-save')).toBeNull();
  expect(core.names()).toContain('project_close');
});

test('FR-PRJ-47 #53 Add… copies the files picked in the native dialog', async () => {
  await openCrypt();
  const handlers = core.handlersSnapshot();
  core.useHandlers({ ...handlers, 'plugin:dialog|open': () => ['D:/ref/statue.png'] });
  fireEvent.click(screen.getByTestId('prj-panel-parts-add'));
  await waitFor(() => expect(files('parts')).toEqual(['statue.png']));
  expect(screen.queryByTestId('prj-panel-parts-add-paths')).toBeNull();
});
