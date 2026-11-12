// PRJ left panel and status segment against the mocked PRJ commands (api/mocks/prj.ts).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';

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

// RGN's active job as the panel reads it (R97): null unless a test sets one.
const rgnJob = vi.hoisted(() => ({ id: null as string | null }));
vi.mock('../region-edit', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../region-edit')>()),
  useActiveJobId: () => rgnJob.id,
}));

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

beforeEach(() => {
  projectStore.reset();
  rgnJob.id = null;
});
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

test('FR-PRJ-23 History Save with nothing changed says so next to the button', async () => {
  await openCrypt();
  expect(screen.queryByTestId('prj-panel-history-unchanged')).toBeNull();
  fireEvent.click(screen.getByTestId('prj-panel-history-save'));
  expect((await screen.findByTestId('prj-panel-history-unchanged')).textContent).toBe(
    'No changes since the last revision',
  );
  // The next commit (a draft save) clears it.
  act(() => core.emit('project:committed', { seq: 9 }));
  expect(screen.queryByTestId('prj-panel-history-unchanged')).toBeNull();
});

test('FR-PRJ-33 FR-PRJ-31 FR-PRJ-32 Storage shows disk use, trash count, next purge, and restores a trashed item', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'results/001 kitchen/02.png',
        kind: 'candidates',
        bytes: 3 * 1024 * 1024,
        trashed_at: '2026-11-01T00:00:00Z',
        restorable: true,
      },
    ],
  });
  const trash = await screen.findByTestId('prj-panel-storage-trash');
  expect(trash.textContent).toContain('Trash: 1 item, 3.0 MB');
  expect(trash.textContent).toContain(
    `next purge ${new Date('2026-12-01T00:00:00Z').toLocaleDateString([], { month: 'short', day: 'numeric' })}`,
  );
  expect(screen.getByTestId('prj-panel-storage-total').textContent).toBe('3.0 MB');
  fireEvent.click(screen.getByTestId('prj-panel-storage-restore'));
  await waitFor(() => expect(screen.queryByTestId('prj-panel-storage-item')).toBeNull());
  expect(core.calls.find((c) => c.cmd === 'project_trash_restore')?.input).toEqual({
    item_id: 'T1',
  });
});

test('FR-PRJ-31 Restore is offered only for restorable trash items (R51)', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'results/001 kitchen/02.png',
        kind: 'candidates',
        bytes: 1024,
        trashed_at: '2026-11-01T00:00:00Z',
        restorable: true,
      },
      {
        id: 'T2',
        path: '.pforge/stray.bin',
        kind: 'orphan',
        bytes: 1024,
        trashed_at: '2026-11-02T00:00:00Z',
        restorable: false,
      },
    ],
  });
  const items = await screen.findAllByTestId('prj-panel-storage-item');
  const row = (id: string) => items.find((i) => i.dataset.recordId === id)!;
  expect(within(row('T1')).queryByTestId('prj-panel-storage-restore')).not.toBeNull();
  expect(within(row('T2')).queryByTestId('prj-panel-storage-restore')).toBeNull();
});

test('BL-30 a multi-file trash item names its first file plus the count, and lists all in the tooltip', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'results/001 kitchen/02.png',
        files: ['results/001 kitchen/02.png', 'results/001 kitchen/03.png'],
        kind: 'candidates',
        bytes: 2048,
        trashed_at: '2026-11-01T00:00:00Z',
        restorable: true,
      },
    ],
  });
  const item = await screen.findByTestId('prj-panel-storage-item');
  expect(item.querySelector('.prj-file-name')?.textContent).toBe('001 · 02.png +1 more');
  expect(item.getAttribute('title')).toBe('results/001 kitchen/02.png\nresults/001 kitchen/03.png');
});

test('walk owner-UX L3: trash rows name the iteration; a superseded mask is labelled, not its id', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'results/002 kitchen/03.png',
        kind: 'candidates',
        bytes: 1024,
        trashed_at: '2026-11-01T00:00:00Z',
        restorable: true,
      },
      {
        id: 'T2',
        path: 'masks/01M6YGFXABCDEFGHJKMNPQRSTV.png',
        kind: 'mask',
        reason: 'superseded',
        bytes: 1024,
        trashed_at: '2026-11-02T00:00:00Z',
        restorable: false,
      },
    ],
  });
  const items = await screen.findAllByTestId('prj-panel-storage-item');
  const names = items.map((i) => i.querySelector('.prj-file-name')?.textContent);
  expect(names).toEqual(['002 · 03.png', 'Mask (superseded)']);
});

test('walk owner-UX L3: Restore replaces the discard notice with a short confirmation', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'results/001 kitchen/02.png',
        kind: 'candidates',
        bytes: 1024,
        trashed_at: '2026-11-01T00:00:00Z',
        restorable: true,
      },
    ],
  });
  act(() =>
    projectStore.setTrashNotice({
      at: 'results',
      text: 'Discarded: in the trash for 30 days (Storage → Restore).',
    }),
  );
  expect(screen.getByTestId('prj-results-notice').textContent).toMatch(/Discarded/);
  fireEvent.click(await screen.findByTestId('prj-panel-storage-restore'));
  expect((await screen.findByTestId('prj-panel-storage-notice')).textContent).toBe(
    'Restored 001 · 02.png.',
  );
  expect(screen.queryByTestId('prj-results-notice')).toBeNull();
  expect(screen.queryByText(/Discarded/)).toBeNull();
});

test('BL-30 a moved folder shows the reason in Storage, not 0 B figures', async () => {
  await openCrypt();
  await screen.findByTestId('prj-panel-storage-usage');
  const moved = () => {
    throw {
      code: 'not_found',
      message: 'The project folder was moved or renamed outside Pocket Forge. Reopen it from home.',
      detail: { reason: 'folder_moved' },
    };
  };
  expect(files('working').length).toBeGreaterThan(0);
  core.useHandlers({
    ...core.handlersSnapshot(),
    project_disk_usage: moved,
    project_trash_list: moved,
    project_folder_list: moved,
  });
  act(() => core.emit('project:committed', { seq: 9 }));
  await waitFor(() => expect(screen.queryByTestId('prj-panel-storage-usage')).toBeNull());
  expect(files('working')).toEqual([]);
  expect(
    screen.getAllByText(
      'The project folder was moved or renamed outside Pocket Forge. Reopen it from home.',
    ).length,
  ).toBeGreaterThanOrEqual(2);
});

test('BL-30 walk owner-UX L1: a Restore failing with folder_moved clears the figures at once', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'results/001 kitchen/02.png',
        kind: 'candidates',
        bytes: 1024,
        trashed_at: '2026-11-01T00:00:00Z',
        restorable: true,
      },
    ],
  });
  await screen.findByTestId('prj-panel-storage-usage');
  const moved = () => {
    throw {
      code: 'not_found',
      message: 'The project folder was moved or renamed outside Pocket Forge. Reopen it from home.',
      detail: { reason: 'folder_moved' },
    };
  };
  core.useHandlers({
    ...core.handlersSnapshot(),
    project_disk_usage: moved,
    project_trash_list: moved,
    project_trash_restore: moved,
  });
  // No commit, no revision change: only the failed call.
  fireEvent.click(await screen.findByTestId('prj-panel-storage-restore'));
  await waitFor(() => expect(screen.queryByTestId('prj-panel-storage-usage')).toBeNull());
  expect(screen.queryByTestId('prj-panel-storage-item')).toBeNull();
  // The banner (and every feature) knows the folder moved.
  const banner = screen
    .getAllByTestId('prj-panel-warning')
    .find((b) => b.dataset.code === 'folder_moved');
  expect(banner).toBeTruthy();
  // The folder came back: the next commit clears the signal (and RGN's Generate block).
  act(() => core.emit('project:committed', { seq: 9 }));
  await waitFor(() =>
    expect(
      screen.queryAllByTestId('prj-panel-warning').some((b) => b.dataset.code === 'folder_moved'),
    ).toBe(false),
  );
});

test('FR-PRJ-31 a failed restore says why as a sentence, not raw core text (walk 2 N2)', async () => {
  await openCrypt({
    trash: [
      {
        id: 'T1',
        path: 'jobs/J1/job (2).json',
        kind: 'conflict_copy',
        bytes: 1024,
        trashed_at: '2026-11-01T00:00:00Z',
        restorable: true,
      },
    ],
  });
  core.useHandlers({
    ...core.handlersSnapshot(),
    project_trash_restore: () => {
      throw { code: 'conflict', message: 'jobs/J1/job.json exists again' };
    },
  });
  fireEvent.click(await screen.findByTestId('prj-panel-storage-restore'));
  expect(await screen.findByText('Jobs/J1/job.json exists again.')).not.toBeNull();
});

test('FR-PRJ-40 a spooled write refused for good shows a banner (BL-25)', async () => {
  await openCrypt();
  const pid = projectStore.get().project!.project_id;
  act(() =>
    core.emit('project:spool-dropped', {
      project_id: 'another-project',
      job_id: 'J1',
      seq: 3,
      op: 'update_job',
      code: 'invalid_input',
    }),
  );
  expect(screen.queryByTestId('prj-error')).toBeNull();
  act(() =>
    core.emit('project:spool-dropped', {
      project_id: pid,
      job_id: 'J1',
      seq: 4,
      op: 'add_candidates',
      code: 'invalid_input',
    }),
  );
  const banner = await screen.findByTestId('prj-error');
  expect(banner.dataset.code).toBe('spool_dropped');
  expect(banner.textContent).toContain('Results of a job');
  expect(banner.textContent).toContain('discarded (invalid input)');
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

test('FR-PRJ-35 FR-PRJ-36 a conflict found mid-session shows the banner and the view resolves it', async () => {
  await openCrypt();
  const side = (which: 'this' | 'other', machine: 'this' | 'other') => ({
    side: which,
    file: which === 'this' ? null : 'project.json',
    readable: true,
    committed_at: '2026-11-07T10:00:00.000Z',
    seq: which === 'this' ? 3 : 9,
    machine,
    layers: [],
    newest_revision: null,
    missing_files: [],
    can_keep: true,
  });
  const p = mock.projects.get(CRYPT)!;
  p.conflict = { source: 'seq', this: side('this', 'this'), other: side('other', 'other') };
  act(() => core.emit('project:conflict', {}));
  const open = await screen.findByTestId('prj-panel-conflict-open');
  // Walk 2 N4: the text and its link don't run together.
  expect(open.closest('[data-code="conflict"]')!.textContent).toBe(
    'This project was also changed on another machine. Compare versions',
  );
  fireEvent.click(open);
  fireEvent.click(await screen.findByTestId('prj-conflict-keep-other'));
  await waitFor(() => expect(screen.queryByTestId('prj-conflict')).toBeNull());
  await waitFor(() => expect(screen.queryByTestId('prj-panel-conflict-open')).toBeNull());
  expect(core.calls.find((c) => c.cmd === 'project_resolve_conflict')?.input).toEqual({
    keep: 'other',
  });
  expect(screen.getByTestId('prj-status-save').dataset.state).toBe('saved');
});

test('walk owner-UX L4 the conflict view has one primary Keep and says "1 file missing"', async () => {
  await openCrypt();
  const side = (which: 'this' | 'other', missing: string[]) => ({
    side: which,
    file: which === 'this' ? null : 'project.json',
    readable: true,
    committed_at: '2026-11-07T10:00:00.000Z',
    seq: which === 'this' ? 3 : 9,
    machine: which,
    layers: [],
    newest_revision: null,
    missing_files: missing,
    can_keep: missing.length === 0,
  });
  const p = mock.projects.get(CRYPT)!;
  p.conflict = {
    source: 'seq',
    this: side('this', []),
    other: side('other', ['patches/01P.png']),
  };
  act(() => core.emit('project:conflict', {}));
  fireEvent.click(await screen.findByTestId('prj-panel-conflict-open'));
  const keepThis = await screen.findByTestId('prj-conflict-keep-this');
  expect(keepThis.classList.contains('primary')).toBe(true);
  expect(screen.getByTestId('prj-conflict-keep-other').classList.contains('primary')).toBe(false);
  expect(screen.getByTestId('prj-conflict-other-syncing').textContent).toBe(
    'Still syncing (1 file missing)',
  );
});

test('FR-PRJ-34 a conflict copy found at open shows Conflict in the status bar, not Saved (walk 2 N3)', async () => {
  await openCrypt({
    warnings: [{ code: 'conflict', message: 'This project was also changed on another machine.' }],
  });
  expect(await screen.findByTestId('prj-panel-conflict-open')).not.toBeNull();
  expect(screen.getByTestId('prj-status-save').dataset.state).toBe('conflict');
});

test('FR-PRJ-21 walkthrough D3: a folder moved while open shows the reason and a banner, Save shows no raw path, Back to home asks', async () => {
  await openCrypt({ pendingSave: true });
  const raw = 'reading C:/pf-mock/library/Maps/Crypt/.pforge/revisions/r.json (os error 3)';
  core.useHandlers({
    ...core.handlersSnapshot(),
    project_save_revision: () =>
      Promise.reject({
        code: 'not_found',
        message:
          'The project folder was moved or renamed outside Pocket Forge. Reopen it from home.',
        detail: { reason: 'folder_moved' },
      }),
  });
  // The background draft save failed (RGN's debounced save doesn't show errors itself).
  act(() => core.emit('project:save-failed', { reason: 'folder_moved', retrying: true }));
  const status = screen.getByTestId('prj-status-save');
  expect(status.dataset.state).toBe('failed');
  expect(status.textContent).toContain('Save failed, project folder moved, retrying');
  const banner = await screen.findByTestId('prj-panel-reopen-home');
  expect(banner.closest('[data-code]')?.getAttribute('data-code')).toBe('folder_moved');

  fireEvent.keyDown(window, { key: 's', ctrlKey: true });
  await waitFor(() => expect(core.names()).toContain('project_save_revision'));
  await waitFor(() => expect(status.textContent).toContain('Save failed, project folder moved'));
  expect(status.textContent).not.toContain('os error');
  expect(document.body.textContent).not.toContain(raw);

  fireEvent.click(screen.getByTestId('prj-panel-reopen-home'));
  expect((await screen.findByTestId('prj-error')).dataset.code).toBe('unsaved');
  expect(projectStore.get().project).not.toBeNull();
  fireEvent.click(screen.getByTestId('prj-panel-close-anyway'));
  await waitFor(() => expect(projectStore.get().project).toBeNull());
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

test('owner #56 Results: each iteration lists its kept candidates with Discard, no confirmation', async () => {
  await openCrypt();
  const base = core.handlersSnapshot();
  const cand = (id: string, index: number, status: string) => ({
    id,
    job_id: 'J1',
    index,
    file: `results/001 kitchen/0${index + 1}.png`,
    status,
    starred: false,
    trash_id: null as string | null,
  });
  const cands = [cand('C1', 0, 'accepted'), cand('C2', 1, 'dismissed'), cand('C3', 2, 'dismissed')];
  core.useHandlers({
    ...base,
    rgn_iterations_list: () => [
      {
        job_id: 'J1',
        n: 1,
        dir: 'results/001 kitchen',
        slug: 'kitchen',
        state: 'closed',
        kept: cands.filter((c) => !c.trash_id).length,
        returned: 3,
        cost: null,
        accepted_in_stack: true,
        references: 0,
        reference_files: [],
      },
    ],
    project_get: async (i) => ({ ...((await base.project_get!(i)) as object), candidates: cands }),
    rgn_candidate_discard: (i) => {
      const c = cands.find((x) => x.id === i.candidate_id)!;
      c.status = 'discarded';
      c.trash_id = 'T9';
      return {};
    },
  });
  act(() => core.emit('project:committed', { seq: 9 }));
  const rows = () =>
    screen.queryAllByTestId('prj-results-candidate').map((r) => r.dataset.recordId);
  await waitFor(() => expect(rows()).toEqual(['C1', 'C2', 'C3']));
  const buttons = screen.getAllByTestId('prj-results-discard') as HTMLButtonElement[];
  expect(buttons[0]!.disabled).toBe(true);
  fireEvent.click(buttons[1]!);
  await waitFor(() => expect(rows()).toEqual(['C1', 'C3']));
  expect(core.calls.find((c) => c.cmd === 'rgn_candidate_discard')?.input).toEqual({
    job_id: 'J1',
    candidate_id: 'C2',
  });
  expect(screen.getByTestId('prj-results-notice').textContent).toMatch(/trash for 30 days/);
  expect(screen.getByTestId('prj-panel-results-item').textContent).toContain('2 / 3');
});

test('FR-RGN-46 #61 Results: Discard all bins the opened iteration but the accepted one, Undo puts it back', async () => {
  await openCrypt();
  const base = core.handlersSnapshot();
  const cand = (id: string, index: number, status: string, starred = false) => ({
    id,
    job_id: 'J1',
    index,
    file: `results/001 kitchen/0${index + 1}.png`,
    status,
    starred,
    trash_id: null as string | null,
  });
  const cands = [
    cand('C1', 0, 'accepted'),
    cand('C2', 1, 'dismissed', true),
    cand('C3', 2, 'dismissed'),
  ];
  let state = 'closed';
  const row = () => ({
    job_id: 'J1',
    n: 1,
    dir: 'results/001 kitchen',
    slug: 'kitchen',
    state,
    kept: cands.filter((c) => !c.trash_id).length,
    returned: 3,
    discardable: cands.filter((c) => !c.trash_id && c.status !== 'accepted').length,
    cost: null,
    accepted_in_stack: true,
    references: 0,
    reference_files: [],
  });
  core.useHandlers({
    ...base,
    rgn_iterations_list: () => [row()],
    project_get: async (i) => ({ ...((await base.project_get!(i)) as object), candidates: cands }),
    rgn_iteration_discard_all: () => {
      for (const c of cands.slice(1)) {
        c.status = 'discarded';
        c.starred = false;
        c.trash_id = 'T1';
      }
      return { trashed: 2, undo_id: 'T1' };
    },
    rgn_iteration_discard_all_undo: () => {
      cands[1]!.starred = true;
      for (const c of cands.slice(1)) {
        c.status = 'dismissed';
        c.trash_id = null;
      }
      return { restored: 2 };
    },
  });
  act(() => core.emit('project:committed', { seq: 9 }));
  const rows = () =>
    screen.queryAllByTestId('prj-results-candidate').map((r) => r.dataset.recordId);
  await waitFor(() => expect(rows()).toEqual(['C1', 'C2', 'C3']));
  const all = () => screen.getByTestId('prj-results-discard-all') as HTMLButtonElement;
  expect(all().disabled).toBe(false);
  fireEvent.click(all());
  // No confirm: one call, the accepted one stays.
  await waitFor(() => expect(rows()).toEqual(['C1']));
  expect(core.calls.filter((c) => c.cmd === 'rgn_iteration_discard_all')).toHaveLength(1);
  expect(core.calls.find((c) => c.cmd === 'rgn_iteration_discard_all')?.input).toEqual({
    job_id: 'J1',
  });
  expect(screen.getByTestId('prj-results-notice').textContent).toMatch(
    /2 results in the trash for 30 days/,
  );
  await waitFor(() => expect(all().disabled).toBe(true));
  fireEvent.click(screen.getByTestId('prj-results-discard-undo'));
  await waitFor(() => expect(rows()).toEqual(['C1', 'C2', 'C3']));
  expect(core.calls.find((c) => c.cmd === 'rgn_iteration_discard_all_undo')?.input).toEqual({
    undo_id: 'T1',
  });
  expect(screen.getByTestId('prj-results-notice').textContent).toMatch(/2 results restored/);
  expect(screen.getAllByTestId('prj-results-candidate')[1]!.textContent).toContain('★');
  // walk-61 L2: a later Discard all (here the tracker's) replaces the restored notice.
  act(() =>
    projectStore.setTrashNotice({ at: 'tracker', text: '2 results in the trash for 30 days' }),
  );
  expect(screen.queryByTestId('prj-results-notice')).toBeNull();
  // Disabled while the set's job runs.
  state = 'running';
  act(() => core.emit('project:committed', { seq: 10 }));
  await waitFor(() => expect(all().disabled).toBe(true));
});

// ------------------------------------------------------------------ FR-PRJ-55 drops (#61)

/** jsdom has no layout: the "element under the pointer" is whatever the test points at. */
let pointedAt: Element | null = null;
document.elementFromPoint = () => pointedAt;
const AT = { x: 40, y: 80 };
const pointAt = (section: string | null) => {
  pointedAt = section ? screen.getByTestId(`prj-panel-${section}`) : document.body;
};
const dragOver = (section: string | null) =>
  act(async () => {
    pointAt(section);
    core.emit('tauri://drag-over', { position: AT });
  });
const dropOn = (section: string | null, paths: string[]) =>
  act(async () => {
    pointAt(section);
    core.emit('tauri://drag-drop', { paths, position: AT });
  });
const zone = (section: string) => screen.getByTestId(`prj-panel-${section}-dropzone`);
const notice = (section: string) => screen.findByTestId(`prj-panel-${section}-drop-notice`);
const dropCalls = () =>
  core.calls.filter((c) => c.cmd === 'project_folder_add' || c.cmd === 'project_base_from_file');

test('FR-PRJ-55 while files are dragged over the project the four folder sections are drop targets and the one under the pointer is lit', async () => {
  await openCrypt();
  for (const s of ['working', 'references', 'structure', 'parts'])
    expect(zone(s).dataset.over).toBeUndefined();
  expect(screen.queryByTestId('prj-panel-results-dropzone')).toBeNull();
  await dragOver('structure');
  expect(screen.getByTestId('prj-panel').dataset.dragging).toBe('true');
  expect(zone('structure').dataset.over).toBe('true');
  expect(zone('working').dataset.over).toBeUndefined();
  await dragOver('working');
  expect(zone('working').dataset.over).toBe('true');
  expect(zone('structure').dataset.over).toBeUndefined();
  // Over History (not a target): nothing is lit, the drag still shows the targets.
  await dragOver('history');
  expect(screen.queryAllByTestId(/-dropzone$/).filter((z) => z.dataset.over)).toEqual([]);
  expect(screen.getByTestId('prj-panel').dataset.dragging).toBe('true');
  await act(async () => core.emit('tauri://drag-leave', {}));
  expect(screen.getByTestId('prj-panel').dataset.dragging).toBeUndefined();
});

test('FR-PRJ-55 FR-PRJ-47 files dropped on References are copied in one by one, the list refreshes and a notice names what was added and skipped', async () => {
  const seed = samplePrjSeed(LIB);
  seed.dirs = { 'D:/art/old sketches': ['D:/art/old sketches/a.png'] };
  const m = prjMock(seed, { emit: core.emit });
  core.useHandlers(m.handlers);
  mock = m.state;
  render(<ProjectPanel />);
  await act(() => projectStore.open(CRYPT));
  await screen.findByTestId('prj-panel');
  await dropOn('references', ['D:/art/mood.jpg', 'D:/art/old sketches', 'D:/art/notes.txt']);
  expect(dropCalls().map((c) => c.input)).toEqual([
    { folder: 'references', paths: ['D:/art/mood.jpg'] },
    { folder: 'references', paths: ['D:/art/old sketches'] },
    { folder: 'references', paths: ['D:/art/notes.txt'] },
  ]);
  await waitFor(() => expect(files('references')).toEqual(['mood.jpg', 'notes.txt']));
  const n = await notice('references');
  expect(n.textContent).toBe(
    'Added to References: mood.jpg, notes.txt. Skipped: old sketches (not a file).',
  );
  expect(n.dataset.kind).toBe('info');
  expect(screen.queryByTestId('prj-panel-structure-drop-notice')).toBeNull();
  // Structure and Parts take drops the same way.
  await dropOn('parts', ['D:/art/door.png']);
  await waitFor(() => expect(files('parts')).toEqual(['door.png']));
  expect((await notice('parts')).textContent).toBe('Added to Parts: door.png.');
});

test('FR-PRJ-55 a drop anywhere else in an open project imports nothing', async () => {
  await openCrypt();
  core.calls.length = 0;
  await dropOn('history', ['D:/art/mood.jpg']);
  await dropOn(null, ['D:/art/mood.jpg']);
  expect(dropCalls()).toEqual([]);
  expect(screen.queryAllByTestId(/-drop-notice$/)).toEqual([]);
});

test('FR-PRJ-55 one image dropped on Working becomes the bottom image as one revision', async () => {
  await openCrypt();
  await dropOn('working', ['D:/art/Lower crypt.png']);
  expect(dropCalls().map((c) => [c.cmd, c.input])).toEqual([
    ['project_base_from_file', { path: 'D:/art/Lower crypt.png' }],
  ]);
  const n = await notice('working');
  expect(n.textContent).toBe('New image dropped: Lower crypt.png.');
  expect(n.dataset.kind).toBe('info');
  await waitFor(() => expect(files('working')).toEqual(['Crypt.png', 'Lower crypt.png']));
  await waitFor(() =>
    expect(projectStore.get().project?.image_path).toBe(`${CRYPT}/working/Lower crypt.png`),
  );
  expect(projectStore.get().project?.layers[0]?.kind).toBe('base');
  const head = screen.getAllByTestId('prj-panel-history-item')[0]!;
  await waitFor(() => expect(head.dataset.reason).toBe('accept'));
});

test('FR-PRJ-55 Working refuses more than one file, a running job, a read-only project and a file that is not an image, changing nothing', async () => {
  await openCrypt({ jobActive: true });
  await dropOn('working', ['D:/art/a.png', 'D:/art/b.png']);
  expect((await notice('working')).textContent).toBe('Drop one image on Working.');
  expect(dropCalls()).toEqual([]);
  await dropOn('working', ['D:/art/a.png']);
  await waitFor(async () =>
    expect((await notice('working')).textContent).toBe(
      'A job is running in this project. Drop the image when it has finished.',
    ),
  );
  expect(files('working')).toEqual(['Crypt.png']);
  cleanup();
  projectStore.reset();

  await openCrypt();
  await dropOn('working', ['D:/art/notes.txt']);
  await waitFor(async () =>
    expect((await notice('working')).textContent).toBe(
      'notes.txt isn’t a PNG, JPEG or WebP image.',
    ),
  );
  expect((await notice('working')).dataset.kind).toBe('error');
  expect(projectStore.get().project?.layers[0]?.kind).toBe('original');
  cleanup();
  projectStore.reset();

  await openCrypt({ read_only: 'newer_version' });
  core.calls.length = 0;
  await dropOn('working', ['D:/art/a.png']);
  expect((await notice('working')).textContent).toBe('The project is read-only.');
  await dropOn('references', ['D:/art/a.png']);
  expect((await notice('references')).textContent).toBe('The project is read-only.');
  expect(dropCalls()).toEqual([]);
});

// ── FR-PRJ-56, 57 (#62): rename and delete from the left panel ─────────────────────────────

const startRename = async () => {
  fireEvent.click(screen.getByTestId('prj-panel-name'));
  return screen.findByTestId<HTMLInputElement>('prj-panel-name-input');
};

test('FR-PRJ-56 the name is edited in place: Enter renames the folder and the project stays open with its history', async () => {
  await openCrypt();
  const before = projectStore.get().project!;
  const input = await startRename();
  expect(input.value).toBe('Crypt');
  fireEvent.change(input, { target: { value: 'Old Crypt' } });
  fireEvent.submit(input.form!);
  await waitFor(() => expect(screen.getByTestId('prj-panel-name').textContent).toBe('Old Crypt'));
  expect(screen.queryByTestId('prj-panel-name-input')).toBeNull();
  expect(core.calls.filter((c) => c.cmd === 'project_rename').map((c) => c.input)).toEqual([
    { path: CRYPT, name: 'Old Crypt' },
  ]);
  expect(mock.openPath).toBe(`${LIB}/Maps/Old Crypt`);
  const after = projectStore.get().project!;
  expect(after.project_id).toBe(before.project_id);
  expect(after.revisions).toEqual(before.revisions);
  // No folder-moved banner, and the save segment still says Saved.
  expect(screen.queryByTestId('prj-panel-warning')).toBeNull();
  expect(screen.queryByTestId('prj-panel-reopen-home')).toBeNull();
  expect(screen.queryByTestId('prj-error')).toBeNull();
  expect(screen.getByTestId('prj-status-save').dataset.state).toBe('saved');
  expect(core.names()).not.toContain('project_close');
});

test('FR-PRJ-56 a taken name shows the error under the name and nothing changes; Esc cancels', async () => {
  await openCrypt();
  const input = await startRename();
  fireEvent.change(input, { target: { value: 'harbour' } });
  fireEvent.submit(input.form!);
  expect((await screen.findByTestId('prj-panel-name-error')).textContent).toContain('taken');
  expect(projectStore.get().project?.name).toBe('Crypt');
  expect(mock.projects.has(CRYPT)).toBe(true);
  // The field has the focus again, so Esc and typing work at once.
  await waitFor(() => expect(document.activeElement).toBe(input));
  // Typing clears the error; Esc leaves the name as it was.
  fireEvent.change(input, { target: { value: 'Vault' } });
  expect(screen.queryByTestId('prj-panel-name-error')).toBeNull();
  fireEvent.keyDown(input, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByTestId('prj-panel-name-input')).toBeNull());
  expect(screen.getByTestId('prj-panel-name').textContent).toBe('Crypt');
  expect(core.calls.filter((c) => c.cmd === 'project_rename')).toHaveLength(1);
});

test('FR-PRJ-56 a running job refuses the rename with its reason; a read-only project can be renamed', async () => {
  await openCrypt({ jobActive: true });
  let input = await startRename();
  fireEvent.change(input, { target: { value: 'Vault' } });
  fireEvent.submit(input.form!);
  expect((await screen.findByTestId('prj-panel-name-error')).textContent).toContain(
    'A job is running',
  );
  expect(projectStore.get().project?.name).toBe('Crypt');
  cleanup();
  projectStore.reset();

  await openCrypt({ read_only: 'newer_version' });
  input = await startRename();
  fireEvent.change(input, { target: { value: 'Vault' } });
  fireEvent.submit(input.form!);
  await waitFor(() => expect(screen.getByTestId('prj-panel-name').textContent).toBe('Vault'));
  expect(projectStore.get().project?.read_only).toBe('newer_version');
});

test('FR-PRJ-57 Delete in the panel asks first: Cancel keeps the project, confirm deletes it and home shows', async () => {
  await openCrypt();
  fireEvent.click(screen.getByTestId('prj-panel-delete'));
  const dialog = await screen.findByTestId('prj-delete-dialog');
  expect(dialog.textContent).toContain('Delete Crypt?');
  expect(dialog.textContent).toContain('It stays in Recently deleted for 30 days.');
  fireEvent.click(screen.getByTestId('prj-delete-cancel'));
  await waitFor(() => expect(screen.queryByTestId('prj-delete-dialog')).toBeNull());
  expect(core.names()).not.toContain('project_delete');
  expect(projectStore.get().project?.name).toBe('Crypt');

  fireEvent.click(screen.getByTestId('prj-panel-delete'));
  fireEvent.click(await screen.findByTestId('prj-delete-confirm'));
  await waitFor(() => expect(projectStore.get().project).toBeNull());
  expect(core.calls.find((c) => c.cmd === 'project_delete')?.input).toEqual({ path: CRYPT });
  expect(screen.queryByTestId('prj-panel')).toBeNull();
  expect(mock.projects.has(CRYPT)).toBe(false);
  expect(mock.trash.map((t) => t.name)).toEqual(['Crypt']);
});

test('FR-PRJ-57 R111 the confirmation comes first: Cancel asks no guard and sends no command', async () => {
  await openCrypt();
  let asked = 0;
  const off = projectStore.addCloseGuard(() => {
    asked++;
    return true;
  });
  try {
    core.calls.length = 0;
    fireEvent.click(screen.getByTestId('prj-panel-delete'));
    await screen.findByTestId('prj-delete-dialog');
    expect(asked).toBe(0);
    fireEvent.click(screen.getByTestId('prj-delete-cancel'));
    await waitFor(() => expect(screen.queryByTestId('prj-delete-dialog')).toBeNull());
    await act(async () => undefined);
    // Nothing was asked (no export was cancelled) and nothing was sent.
    expect(asked).toBe(0);
    expect(core.names()).toEqual([]);
    expect(projectStore.get().project?.name).toBe('Crypt');
  } finally {
    off();
  }
});

test('FR-PRJ-57 R111 the close questions come only after Confirm, with the confirmation out of their way', async () => {
  await openCrypt();
  // EXP's "Cancel the export and close?" holds the close guard until it is answered.
  let answer: { proceed: () => void; cancel: () => void } | null = null;
  const reasons: string[] = [];
  const off = projectStore.addCloseGuard((proceed, cancel, why) => {
    answer = { proceed, cancel };
    reasons.push(why);
    return false;
  });
  try {
    fireEvent.click(screen.getByTestId('prj-panel-delete'));
    await screen.findByTestId('prj-delete-dialog');
    expect(answer).toBeNull();
    fireEvent.click(screen.getByTestId('prj-delete-confirm'));
    await waitFor(() => expect(answer).not.toBeNull());
    // The modal confirmation is gone, so the question in the export panel can be answered.
    expect(screen.queryByTestId('prj-delete-dialog')).toBeNull();
    expect(core.names()).not.toContain('project_delete');
    // "Keep exporting": nothing is deleted and the project stays open.
    await act(async () => answer!.cancel());
    expect(core.names()).not.toContain('project_delete');
    expect(screen.queryByTestId('prj-delete-dialog')).toBeNull();
    expect(projectStore.get().project?.name).toBe('Crypt');

    answer = null;
    fireEvent.click(screen.getByTestId('prj-panel-delete'));
    fireEvent.click(await screen.findByTestId('prj-delete-confirm'));
    await waitFor(() => expect(answer).not.toBeNull());
    await act(async () => answer!.proceed());
    await waitFor(() => expect(projectStore.get().project).toBeNull());
  } finally {
    off();
  }
  expect(reasons).toEqual(['close', 'close']);
  expect(core.names()).toContain('project_delete');
});

test('FR-PRJ-57 R97 while a job runs Delete is disabled and says why, without opening the confirmation', async () => {
  rgnJob.id = 'job-1';
  await openCrypt({ jobActive: true });
  const del = screen.getByTestId<HTMLButtonElement>('prj-panel-delete');
  expect(del.disabled).toBe(true);
  expect(del.title).toContain('A job is running');
  fireEvent.click(del);
  expect(screen.queryByTestId('prj-delete-dialog')).toBeNull();
  expect(core.names()).not.toContain('project_delete');
});

test('FR-PRJ-57 a refused delete says why in the confirmation and the project stays open', async () => {
  await openCrypt({ jobActive: true });
  fireEvent.click(screen.getByTestId('prj-panel-delete'));
  fireEvent.click(await screen.findByTestId('prj-delete-confirm'));
  expect((await screen.findByTestId('prj-delete-error')).textContent).toContain('A job is running');
  expect(projectStore.get().project?.name).toBe('Crypt');
  expect(mock.trash).toEqual([]);
  fireEvent.click(screen.getByTestId('prj-delete-cancel'));
  await waitFor(() => expect(screen.queryByTestId('prj-delete-dialog')).toBeNull());
  expect(screen.getByTestId('prj-panel')).toBeTruthy();
});
