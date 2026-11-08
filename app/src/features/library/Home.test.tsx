// PRJ home grid, rail and creation against the mocked PRJ commands (api/mocks/prj.ts).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';

import { prjMock, samplePrjSeed, type PrjMockSeed, type PrjMockState } from '../../api/mocks/prj';
import { installTestCore } from '../../api/mocks/testCore';
import { projectStore } from '../../store/project';
import { Home } from './Home';
import { Rail } from './Rail';
import { libraryStore } from './store';

const core = installTestCore();
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
    'Drop an image to start a item',
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
  fireEvent.click(screen.getByTestId('prj-home-new'));
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

test('FR-PRJ-02 an unavailable root shows Library unavailable with Choose again, and creates nothing', async () => {
  setup({ unavailable: true });
  expect((await screen.findByTestId('prj-library-unavailable')).textContent).toContain(LIB);
  expect(screen.getByTestId<HTMLButtonElement>('prj-home-new').disabled).toBe(true);
  await drop(['D:/maps/x.png']);
  expect(cards()).toHaveLength(0);
  fireEvent.click(screen.getByTestId('prj-library-choose-again'));
  expect(await screen.findByTestId('prj-setup')).toBeTruthy();
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
