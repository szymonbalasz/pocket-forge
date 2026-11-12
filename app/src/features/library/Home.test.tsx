// PRJ home grid, rail and creation against the mocked PRJ commands (api/mocks/prj.ts).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';

import { prjMock, samplePrjSeed, type PrjMockSeed, type PrjMockState } from '../../api/mocks/prj';
import { installTestCore } from '../../api/mocks/testCore';
import { generateTab } from '../../shared/generateTabs';
import { projectStore } from '../../store/project';
import { Home } from './Home';
import { Rail } from './Rail';
import { libraryStore } from './store';

const core = installTestCore();

// Radix menus measure themselves (Popper): jsdom has no ResizeObserver.
(globalThis as unknown as Record<string, unknown>).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
let mock: PrjMockState;
const LIB = 'C:/pf-mock/library';

function setup(patch: Partial<PrjMockSeed> = {}) {
  const m = prjMock({ ...samplePrjSeed(LIB), ...patch }, { emit: core.emit });
  core.useHandlers(m.handlers);
  mock = m.state;
  return render(
    <>
      <nav>
        <Rail />
      </nav>
      <Home />
    </>,
  );
}

const cards = () => screen.queryAllByTestId('prj-home-card');
const cardNames = () => cards().map((c) => within(c).getByTestId('prj-home-card-name').textContent);
/** OQ-GEN-10: + New is a menu; `image` = From image…, `generate` = Generate new…. */
async function newMenu(item: 'image' | 'generate') {
  fireEvent.pointerDown(screen.getByTestId('prj-home-new'), {
    button: 0,
    ctrlKey: false,
    pointerType: 'mouse',
  });
  fireEvent.click(await screen.findByTestId(`prj-home-new-${item}`));
}
const drop = (paths: string[]) =>
  act(async () => {
    core.emit('tauri://drag-drop', { paths, position: { x: 0, y: 0 } });
  });

beforeEach(() => {
  projectStore.reset();
  libraryStore.reset();
});
afterEach(cleanup);

test('FR-PRJ-49 the home grid lists the section’s cards newest first with thumbnail, name, iterations, last edited and badges', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  expect(cardNames()).toEqual(['Harbour', 'Crypt']);
  const [harbour, crypt] = cards() as [HTMLElement, HTMLElement];
  expect(within(crypt).getByTestId('prj-home-card-iterations').textContent).toBe('3 iterations');
  expect(within(harbour).getByTestId('prj-home-card-iterations').textContent).toBe('1 iteration');
  expect(within(crypt).getByTestId('prj-home-card-thumb').tagName).toBe('IMG');
  expect(within(harbour).getByTestId('prj-home-card-thumb').tagName).toBe('SPAN');
  expect(within(crypt).getByTestId('prj-home-card-edited').getAttribute('title')).toBe(
    '2026-11-07T10:00:00.000Z',
  );
  expect(within(harbour).getByTestId('prj-home-card-badge').dataset.badge).toBe('read_only');
  expect(crypt.dataset.recordId).toMatch(/^01JMOCKPRJ/);
  expect(core.calls.find((c) => c.cmd === 'library_list_projects')?.input).toEqual({
    section: 'maps',
  });
});

test('FR-PRJ-49 section switching: the rail picks the section the home grid shows; an empty section says how to start', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  expect(screen.getByTestId('prj-rail-maps').getAttribute('aria-current')).toBe('page');

  fireEvent.click(screen.getByTestId('prj-rail-tokens'));
  await waitFor(() => expect(cardNames()).toEqual(['Goblin']));
  expect(screen.getByTestId('prj-rail-tokens').getAttribute('aria-current')).toBe('page');
  expect(screen.getByTestId('prj-rail-maps').getAttribute('aria-current')).toBeNull();
  expect(screen.getByTestId('prj-home-title').textContent).toBe('Tokens');

  fireEvent.click(screen.getByTestId('prj-rail-items'));
  await waitFor(() => expect(screen.getByTestId('prj-home-empty')).toBeTruthy());
  expect(screen.getByTestId('prj-home-empty').textContent).toContain(
    'Drop an image to start an item',
  );
  expect(cards()).toHaveLength(0);
});

test('FR-PRJ-03 FR-PRJ-06 a drop on the home grid creates projects in the selected section and summarises skips', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  await drop(['D:/maps/Sewer.png', 'D:/maps/cave.jpg', 'D:/maps/art.psd']);
  await waitFor(() =>
    expect(screen.getByTestId('prj-import-summary-text').textContent).toBe(
      '2 created · 1 skipped (unsupported)',
    ),
  );
  await waitFor(() => expect(cards()).toHaveLength(4));
  expect(cardNames()).toEqual(expect.arrayContaining(['Sewer', 'cave']));
  const create = core.calls.find((c) => c.cmd === 'project_create');
  expect(create?.input).toEqual({
    section: 'maps',
    paths: ['D:/maps/Sewer.png', 'D:/maps/cave.jpg', 'D:/maps/art.psd'],
  });
  const skipped = screen.getAllByTestId('prj-import-summary-item');
  expect(skipped.map((s) => s.dataset.status)).toEqual(['skipped']);
  expect(skipped[0]!.textContent).toContain('art.psd');
});

test('FR-PRJ-08 dropping the same source again reports already in library with Open; a taken name gets (2)', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  await drop(['D:/other/Crypt.png']);
  await waitFor(() => expect(cardNames()).toContain('Crypt (2)'));
  await drop(['D:/other/Crypt.png']);
  await waitFor(() =>
    expect(screen.getByTestId('prj-import-summary-text').textContent).toBe('1 already in library'),
  );
  fireEvent.click(screen.getByTestId('prj-import-summary-open'));
  await waitFor(() => expect(projectStore.get().project?.name).toBe('Crypt (2)'));
});

test('FR-PRJ-07 a dropped folder makes one project per image found recursively', async () => {
  setup();
  fireEvent.click(await screen.findByTestId('prj-rail-tokens'));
  await waitFor(() => expect(cards()).toHaveLength(1));
  await drop(['D:/art/tokens']);
  await waitFor(() => expect(cards()).toHaveLength(3));
  expect(cardNames()).toEqual(expect.arrayContaining(['orc', 'elf', 'Goblin']));
});

test('FR-PRJ-03 FR-PRJ-10 pasting image data creates a pasted-<time> project; a pasted path list creates projects', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  const png = new File([new Uint8Array([137, 80, 78, 71])], 'image.png', { type: 'image/png' });
  const paste = (data: { items?: unknown[]; text?: string }) => {
    const ev = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent;
    Object.defineProperty(ev, 'clipboardData', {
      value: { items: data.items ?? [], getData: () => data.text ?? '' },
    });
    document.body.dispatchEvent(ev);
  };
  await act(async () =>
    paste({ items: [{ kind: 'file', type: 'image/png', getAsFile: () => png }] }),
  );
  await waitFor(() => expect(cardNames()).toContain('pasted-20261108-120000'));
  const clip = core.calls.find((c) => c.cmd === 'project_create_from_clipboard');
  expect(clip?.input).toEqual({ section: 'maps', data_b64: 'iVBORw==' });

  await act(async () => paste({ text: '"D:\\maps\\Moor.png"\r\n"D:\\maps\\Fen.webp"' }));
  await waitFor(() => expect(cardNames()).toEqual(expect.arrayContaining(['Moor', 'Fen'])));
});

test('FR-PRJ-03 + New takes several paths and creates them', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  await newMenu('image');
  fireEvent.change(await screen.findByTestId('prj-home-new-paths'), {
    target: { value: 'D:/a/One.png\nD:/a/Two.png' },
  });
  fireEvent.click(screen.getByTestId('prj-home-new-create'));
  await waitFor(() => expect(cards()).toHaveLength(4));
});

test('FR-PRJ-11 creation shows done / total and Cancel keeps the finished projects', async () => {
  setup({ createDelayMs: 30 });
  await waitFor(() => expect(cards()).toHaveLength(2));
  await drop(['D:/a/1.png', 'D:/a/2.png', 'D:/a/3.png', 'D:/a/4.png', 'D:/a/5.png']);
  const progress = await screen.findByTestId('prj-import-progress');
  await waitFor(() => expect(progress.textContent).toMatch(/Creating [12] \/ 5/));
  fireEvent.click(screen.getByTestId('prj-import-cancel'));
  await waitFor(() =>
    expect(screen.getByTestId('prj-import-summary-text').textContent).toMatch(
      /^\d created · canceled$/,
    ),
  );
  const created = cards().length - 2;
  expect(created).toBeGreaterThan(0);
  expect(created).toBeLessThan(5);
  expect(core.names()).toContain('project_create_cancel');
});

test('FR-PRJ-01 first run asks for the library folder and shows field errors', async () => {
  setup({ configured: false });
  const input = await screen.findByTestId('prj-setup-root-path');
  fireEvent.change(input, { target: { value: 'E:/missing' } });
  fireEvent.click(screen.getByTestId('prj-setup-pick-root'));
  expect((await screen.findByTestId('prj-setup-error')).textContent).toBe(
    'That folder does not exist',
  );
  fireEvent.change(input, { target: { value: LIB } });
  fireEvent.click(screen.getByTestId('prj-setup-pick-root'));
  await waitFor(() => expect(cards()).toHaveLength(2));
  expect(screen.queryByTestId('prj-setup')).toBeNull();
  expect(core.calls.find((c) => c.cmd === 'library_set_roots')?.input).toEqual({
    roots: ['E:/missing'],
  });
});

test('FR-PRJ-02 an unavailable root shows Library unavailable with Choose again (prefilled), and creates nothing', async () => {
  setup({ unavailable: true });
  expect((await screen.findByTestId('prj-library-unavailable')).textContent).toContain(LIB);
  expect(screen.getByTestId<HTMLButtonElement>('prj-home-new').disabled).toBe(true);
  await drop(['D:/maps/x.png']);
  expect(cards()).toHaveLength(0);
  fireEvent.click(screen.getByTestId('prj-library-choose-again'));
  expect(await screen.findByTestId('prj-setup')).toBeTruthy();
  // It starts from the current root, not an empty field (BL-27).
  expect(screen.getByTestId<HTMLInputElement>('prj-setup-root-path').value).toBe(LIB);
  mock.setUnavailable(false);
});

test('FR-PRJ-14 clicking a card opens the project folder and hides the home grid', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  const crypt = cards()[1]!;
  fireEvent.click(within(crypt).getByRole('button'));
  await waitFor(() => expect(screen.queryByTestId('prj-home-dropzone')).toBeNull());
  expect(core.calls.find((c) => c.cmd === 'project_open')?.input).toEqual({
    path: `${LIB}/Maps/Crypt`,
  });
  expect(projectStore.get().project?.kind).toBe('map');
  // The rail goes back home: the project closes.
  fireEvent.click(screen.getByTestId('prj-rail-maps'));
  await waitFor(() => expect(screen.getByTestId('prj-home-dropzone')).toBeTruthy());
  expect(core.names()).toContain('project_close');
});

test('FR-PRJ-45 rename from the card: Enter renames the folder, a taken name shows the error, Esc cancels', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  const crypt = () => cards().find((c) => c.dataset.path?.endsWith('/Crypt'))!;
  fireEvent.keyDown(within(crypt()).getByRole('button'), { key: 'F2' });
  let input = await screen.findByTestId<HTMLInputElement>('prj-home-card-rename-input');
  fireEvent.change(input, { target: { value: 'harbour' } });
  fireEvent.submit(input.form!);
  expect((await screen.findByTestId('prj-home-card-rename-error')).textContent).toContain('taken');
  fireEvent.keyDown(input, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByTestId('prj-home-card-rename-input')).toBeNull());

  fireEvent.keyDown(within(crypt()).getByRole('button'), { key: 'F2' });
  input = await screen.findByTestId<HTMLInputElement>('prj-home-card-rename-input');
  fireEvent.change(input, { target: { value: 'Old Crypt' } });
  fireEvent.submit(input.form!);
  await waitFor(() => expect(cardNames()).toContain('Old Crypt'));
  expect(core.calls.filter((c) => c.cmd === 'project_rename').map((c) => c.input)).toEqual([
    { path: `${LIB}/Maps/Crypt`, name: 'harbour' },
    { path: `${LIB}/Maps/Crypt`, name: 'Old Crypt' },
  ]);
  expect(mock.projects.has(`${LIB}/Maps/Old Crypt`)).toBe(true);
});

test('FR-PRJ-49 a late listing (a thumbnail refresh) never replaces a newer one (a rename)', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  const h = core.handlersSnapshot();
  const list = h.library_list_projects!;
  let release: () => void = () => undefined;
  let first = true;
  core.useHandlers({
    ...h,
    library_list_projects: (input) => {
      const cardsNow = list(input); // the listing as it is now…
      if (!first) return cardsNow;
      first = false;
      return new Promise<void>((r) => (release = r)).then(() => cardsNow); // …answered late
    },
  });
  act(() => core.emit('project:thumb-updated', { project_id: 'x' }));
  await waitFor(() => expect(core.names()).toContain('library_list_projects'));
  const crypt = libraryStore.get().cards!.find((c) => c.name === 'Crypt')!;
  await act(() => libraryStore.rename(crypt, 'Old Crypt'));
  await waitFor(() => expect(cardNames()).toContain('Old Crypt'));
  await act(async () => release());
  await act(() => new Promise((r) => setTimeout(r, 20)));
  expect(cardNames()).toContain('Old Crypt');
  expect(cardNames()).not.toContain('Crypt');
});

test('FR-PRJ-50 a thumbnail that failed to load is tried again when a new one lands', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  const crypt = () => cards().find((c) => c.dataset.path?.endsWith('/Crypt'))!;
  const thumb = () => within(crypt()).getByTestId('prj-home-card-thumb');
  fireEvent.error(thumb()); // half written when it was read
  expect(thumb().tagName).toBe('SPAN');
  const h = core.handlersSnapshot();
  const list = h.library_list_projects!;
  core.useHandlers({
    ...h,
    // The rewritten thumbnail has a new URL (its key hashes the file's size and mtime).
    library_list_projects: async (input) =>
      ((await list(input)) as { thumb_url: string | null }[]).map((c) => ({
        ...c,
        thumb_url: c.thumb_url && `${c.thumb_url}?v=2`,
      })),
  });
  act(() => core.emit('project:thumb-updated', { project_id: 'x' }));
  await waitFor(() => expect(thumb().tagName).toBe('IMG'));
  expect(thumb().getAttribute('src')).toMatch(/\?v=2$/);
});

test('FR-PRJ-17 a cloud-only project offline shows not available offline with Retry, which opens it once online', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  let offline = true;
  const handlers = prjMock(samplePrjSeed(LIB), { emit: core.emit }).handlers;
  core.useHandlers({
    ...handlers,
    project_open: (input) =>
      offline
        ? Promise.reject({ code: 'not_available_offline', message: 'offline' })
        : handlers.project_open!(input),
  });
  const crypt = cards().find((c) => c.dataset.path?.endsWith('/Crypt'))!;
  fireEvent.click(within(crypt).getByRole('button'));
  const banner = await screen.findByTestId('prj-error');
  expect(banner.dataset.code).toBe('not_available_offline');
  expect(banner.textContent).toContain('Not available offline');
  offline = false;
  fireEvent.click(screen.getByTestId('prj-error-retry'));
  await waitFor(() => expect(projectStore.get().project?.name).toBe('Crypt'));
});

test('FR-PRJ-49 R45 an unreadable project card has no last-edited time, sorts last and shows the badge', async () => {
  const seed = samplePrjSeed(LIB);
  seed.projects.push({
    section: 'maps',
    name: 'Broken',
    last_edited: '2026-11-08T11:00:00.000Z',
    unreadable: true,
  });
  setup({ projects: seed.projects });
  await waitFor(() => expect(cards()).toHaveLength(3));
  expect(cardNames()).toEqual(['Harbour', 'Crypt', 'Broken']);
  const broken = cards()[2]!;
  expect(within(broken).getByTestId('prj-home-card-edited').textContent).toBe('');
  expect(within(broken).getByTestId('prj-home-card-badge').dataset.badge).toBe('unreadable');
});

test('FR-PRJ-03 #53 + New opens the native image dialog and creates the picked files; Cancel creates nothing', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  const handlers = prjMock(samplePrjSeed(LIB), { emit: core.emit }).handlers;
  let picked: string[] | null = ['D:/a/One.png', 'D:/a/Two.png'];
  core.useHandlers({ ...handlers, 'plugin:dialog|open': () => picked });
  await newMenu('image');
  await waitFor(() => expect(cardNames()).toEqual(expect.arrayContaining(['One', 'Two'])));
  expect(screen.queryByTestId('prj-home-new-paths')).toBeNull();
  const opts = core.calls.find((c) => c.cmd === 'plugin:dialog|open')?.input.options as Record<
    string,
    unknown
  >;
  expect(opts).toMatchObject({
    multiple: true,
    directory: false,
    filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }],
  });

  picked = null;
  const before = cards().length;
  await newMenu('image');
  await waitFor(() =>
    expect(core.names().filter((n) => n === 'plugin:dialog|open')).toHaveLength(2),
  );
  expect(screen.queryByTestId('prj-home-new-paths')).toBeNull();
  expect(cards()).toHaveLength(before);
});

test('FR-PRJ-01 #53 Browse… picks the library folder with the native folder dialog', async () => {
  setup({ configured: false });
  await screen.findByTestId('prj-setup');
  const handlers = prjMock(
    { ...samplePrjSeed(LIB), configured: false },
    { emit: core.emit },
  ).handlers;
  core.useHandlers({ ...handlers, 'plugin:dialog|open': () => LIB });
  fireEvent.click(screen.getByTestId('prj-setup-browse'));
  await waitFor(() => expect(cards()).toHaveLength(2));
  expect(core.calls.find((c) => c.cmd === 'plugin:dialog|open')?.input.options).toMatchObject({
    directory: true,
  });
  expect(core.calls.find((c) => c.cmd === 'library_set_roots')?.input).toEqual({ roots: [LIB] });
});

test('FR-GEN-01 OQ-GEN-10 + New → Generate new…: the name is prefilled per section and made free; the project is created and opens on New image', async () => {
  setup({
    projects: [
      ...samplePrjSeed(LIB).projects,
      { section: 'maps', name: 'New map', last_edited: '2026-11-01T10:00:00.000Z' },
    ],
  });
  await waitFor(() => expect(cards()).toHaveLength(3));
  await newMenu('generate');
  const name = await screen.findByTestId<HTMLInputElement>('gen-new-name');
  expect(name.value).toBe('New map (2)');

  // A taken name is refused inline (FR-PRJ-44).
  fireEvent.change(name, { target: { value: 'crypt' } });
  await act(async () => fireEvent.click(screen.getByTestId('gen-new-ok')));
  expect((await screen.findByTestId('gen-new-error')).textContent).toContain('already exists');

  fireEvent.change(name, { target: { value: 'Flooded crypt' } });
  await act(async () => fireEvent.click(screen.getByTestId('gen-new-ok')));
  await waitFor(() => expect(projectStore.get().project?.name).toBe('Flooded crypt'));
  expect(core.calls.filter((c) => c.cmd === 'gen_project_create').at(-1)?.input).toEqual({
    section: 'maps',
    name: 'Flooded crypt',
  });
  // FR-GEN-02/03: no image yet.
  expect(projectStore.get().project?.image_path).toBeNull();
  expect(projectStore.get().project?.layers).toEqual([]);
  expect(screen.queryByTestId('gen-new')).toBeNull();
  // The Generate area opens it on New image (FR-GEN-01).
  generateTab.projectOpened();
  expect(generateTab.get()).toBe('new-image');
  generateTab.select('region');
});

test('FR-GEN-01 Generate new… prefills New token / New item by section; Cancel creates nothing', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  fireEvent.click(screen.getByTestId('prj-rail-tokens'));
  await waitFor(() => expect(cardNames()).toEqual(['Goblin']));
  await newMenu('generate');
  expect((await screen.findByTestId<HTMLInputElement>('gen-new-name')).value).toBe('New token');
  fireEvent.click(screen.getByTestId('gen-new-cancel'));
  await waitFor(() => expect(screen.queryByTestId('gen-new')).toBeNull());
  fireEvent.click(screen.getByTestId('prj-rail-items'));
  await waitFor(() => expect(screen.getByTestId('prj-home-empty')).toBeTruthy());
  await newMenu('generate');
  expect((await screen.findByTestId<HTMLInputElement>('gen-new-name')).value).toBe('New item');
  expect(core.names()).not.toContain('gen_project_create');
});

// ── FR-PRJ-57, 58 (#62): delete from the card, Recently deleted, Restore ───────────────────

const trashNames = () =>
  screen
    .queryAllByTestId('prj-trash-item')
    .map((i) => within(i).getByTestId('prj-trash-name').textContent);
const cardOf = (name: string) => cards().find((c) => c.dataset.path?.endsWith(`/${name}`))!;
async function deleteFromCard(name: string) {
  fireEvent.contextMenu(cardOf(name));
  fireEvent.click(await screen.findByTestId('prj-home-card-delete'));
  return screen.findByTestId('prj-delete-dialog');
}

test('FR-PRJ-57 Delete in the card’s context menu asks first; Cancel keeps the card, confirm moves it to Recently deleted', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  expect(screen.queryByTestId('prj-trash-list')).toBeNull();

  let dialog = await deleteFromCard('Crypt');
  expect(dialog.textContent).toContain('Delete Crypt?');
  expect(dialog.textContent).toContain('It stays in Recently deleted for 30 days.');
  fireEvent.click(screen.getByTestId('prj-delete-cancel'));
  await waitFor(() => expect(screen.queryByTestId('prj-delete-dialog')).toBeNull());
  expect(core.names()).not.toContain('project_delete');
  expect(cardNames()).toEqual(['Harbour', 'Crypt']);

  dialog = await deleteFromCard('Crypt');
  fireEvent.click(within(dialog).getByTestId('prj-delete-confirm'));
  await waitFor(() => expect(cardNames()).toEqual(['Harbour']));
  expect(core.calls.find((c) => c.cmd === 'project_delete')?.input).toEqual({
    path: `${LIB}/Maps/Crypt`,
  });
  await waitFor(() => expect(trashNames()).toEqual(['Crypt']));
  expect(screen.queryByTestId('prj-delete-dialog')).toBeNull();
});

test('FR-PRJ-57 a refused delete shows the reason in the confirmation and the card stays', async () => {
  const seed = samplePrjSeed(LIB);
  seed.projects[0] = { ...seed.projects[0]!, jobActive: true };
  setup({ projects: seed.projects });
  await waitFor(() => expect(cards()).toHaveLength(2));
  const dialog = await deleteFromCard('Crypt');
  fireEvent.click(within(dialog).getByTestId('prj-delete-confirm'));
  expect((await screen.findByTestId('prj-delete-error')).textContent).toContain('A job is running');
  expect(cardNames()).toEqual(['Harbour', 'Crypt']);
  expect(mock.trash).toEqual([]);
});

test('FR-PRJ-58 Recently deleted lists the section’s deleted projects newest first with the purge date; Restore brings the card back', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  await act(() => libraryStore.remove(libraryStore.get().cards!.find((c) => c.name === 'Crypt')!));
  await act(() =>
    libraryStore.remove(libraryStore.get().cards!.find((c) => c.name === 'Harbour')!),
  );
  await waitFor(() => expect(trashNames()).toEqual(['Harbour', 'Crypt']));
  expect(core.calls.filter((c) => c.cmd === 'library_trash_list').at(-1)?.input).toEqual({
    section: 'maps',
  });
  // An empty section still shows them, under the empty hint.
  expect(screen.getByTestId('prj-home-empty')).toBeTruthy();
  const [harbour] = screen.getAllByTestId('prj-trash-item');
  expect(harbour!.dataset.recordId).toBe(String(mock.trash[1]!.id));
  // Deleted 8 Nov 2026 → purged 30 days later.
  expect(within(harbour!).getByTestId('prj-trash-until').textContent).toMatch(/^until .*8/);

  // Another section has its own list.
  fireEvent.click(screen.getByTestId('prj-rail-tokens'));
  await waitFor(() => expect(cardNames()).toEqual(['Goblin']));
  expect(screen.queryByTestId('prj-trash-list')).toBeNull();
  fireEvent.click(screen.getByTestId('prj-rail-maps'));
  await waitFor(() => expect(trashNames()).toEqual(['Harbour', 'Crypt']));

  const crypt = screen.getAllByTestId('prj-trash-item')[1]!;
  fireEvent.click(within(crypt).getByTestId('prj-trash-restore'));
  await waitFor(() => expect(cardNames()).toEqual(['Crypt']));
  expect(trashNames()).toEqual(['Harbour']);
  expect(core.calls.find((c) => c.cmd === 'library_trash_restore')?.input).toEqual({
    id: crypt.dataset.recordId,
  });
  expect(mock.projects.has(`${LIB}/Maps/Crypt`)).toBe(true);
});

test('FR-PRJ-58 a restore whose name is taken comes back with a suffix; a failed one shows the error and stays listed', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  await act(() => libraryStore.remove(libraryStore.get().cards!.find((c) => c.name === 'Crypt')!));
  await waitFor(() => expect(trashNames()).toEqual(['Crypt']));
  // The name is taken by the time it is restored.
  await drop(['D:/maps/Crypt.png']);
  await waitFor(() => expect(cardNames()).toContain('Crypt'));
  fireEvent.click(screen.getByTestId('prj-trash-restore'));
  await waitFor(() => expect(cardNames()).toContain('Crypt (2)'));
  expect(screen.queryByTestId('prj-trash-list')).toBeNull();

  await act(() =>
    libraryStore.remove(libraryStore.get().cards!.find((c) => c.name === 'Harbour')!),
  );
  await waitFor(() => expect(trashNames()).toEqual(['Harbour']));
  (mock.trash[0]!.project as Record<string, unknown>).folderLocked = true;
  fireEvent.click(screen.getByTestId('prj-trash-restore'));
  expect((await screen.findByTestId('prj-error')).dataset.code).toBe('locked');
  expect(trashNames()).toEqual(['Harbour']);
});

test('FR-PRJ-58 the library trash purge at startup reloads Recently deleted', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  await act(() => libraryStore.remove(libraryStore.get().cards!.find((c) => c.name === 'Crypt')!));
  await waitFor(() => expect(trashNames()).toEqual(['Crypt']));
  mock.trash.length = 0;
  // A project's own purge (it names its project) is not the library's.
  await act(async () => core.emit('trash:purged', { project_id: 'p1', count: 1, bytes: 10 }));
  expect(trashNames()).toEqual(['Crypt']);
  await act(async () => core.emit('trash:purged', { count: 1, bytes: 4096 }));
  await waitFor(() => expect(screen.queryByTestId('prj-trash-list')).toBeNull());
});
