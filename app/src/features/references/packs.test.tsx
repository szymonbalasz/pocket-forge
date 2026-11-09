// REF packs home and pack view against the mocked REF commands (api/mocks/ref.ts).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';

import { prjMock, samplePrjSeed } from '../../api/mocks/prj';
import { refMock, sampleRefSeed, type RefMockSeed, type RefMockState } from '../../api/mocks/ref';
import { installTestCore } from '../../api/mocks/testCore';
import { homeView } from '../../shared/home';
import { projectStore } from '../../store/project';
import { libraryFeature } from '../library';
import { ReferencesHome } from './ReferencesHome';
import { refs } from './store';

const core = installTestCore();
let mock: RefMockState;
const Rail = libraryFeature.slots.rail!;

function polyfill(): void {
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => undefined;
  proto.releasePointerCapture ??= () => undefined;
  proto.scrollIntoView ??= () => undefined;
  (globalThis as Record<string, unknown>).ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

function setup(patch: Partial<RefMockSeed> = {}, view: 'projects' | 'references' = 'references') {
  const prj = prjMock(samplePrjSeed(), { emit: core.emit });
  const m = refMock({ ...sampleRefSeed(), ...patch }, { emit: core.emit });
  core.useHandlers({ ...prj.handlers, ...m.handlers });
  mock = m.state;
  homeView.set(view);
  return render(
    <>
      <nav>
        <Rail />
      </nav>
      <ReferencesHome />
    </>,
  );
}

const cards = () => screen.queryAllByTestId('ref-home-card');
const cardNames = () => cards().map((c) => within(c).getByTestId('ref-home-card-name').textContent);
const card = (name: string) =>
  cards().find((c) => within(c).getByTestId('ref-home-card-name').textContent === name)!;
const images = () =>
  screen.queryAllByTestId('ref-pack-image').map((i) => i.getAttribute('data-record-id'));
const drop = (paths: string[]) =>
  act(async () => {
    core.emit('tauri://drag-drop', { paths, position: { x: 0, y: 0 } });
  });
const WEAPONS = '01JMOCKPACKWEAPONS00000000';

async function openWeapons() {
  await waitFor(() => expect(cards()).toHaveLength(2));
  fireEvent.click(within(card('Weapons')).getByRole('button'));
  await waitFor(() => expect(images()).toHaveLength(3));
}

async function nameDialog(name: string) {
  const input = await screen.findByTestId('ref-dialog-name-input');
  fireEvent.change(input, { target: { value: name } });
  fireEvent.click(screen.getByTestId('ref-dialog-name-submit'));
}

beforeEach(() => {
  polyfill();
  projectStore.reset();
  refs.reset();
  homeView.set('projects');
});
afterEach(cleanup);

test('FR-PRJ-52 the rail has a References entry between Items and Settings that shows the packs home', async () => {
  setup({}, 'projects');
  const rail = screen.getAllByRole('button').map((b) => b.dataset.testid);
  expect(rail.indexOf('prj-rail-references')).toBe(rail.indexOf('prj-rail-items') + 1);
  expect(rail.indexOf('prj-rail-settings')).toBe(rail.indexOf('prj-rail-references') + 1);
  expect(screen.queryByTestId('ref-home-dropzone')).toBeNull();

  fireEvent.click(screen.getByTestId('prj-rail-references'));
  await waitFor(() => expect(cards()).toHaveLength(2));
  expect(screen.getByTestId('prj-rail-references').getAttribute('aria-current')).toBe('page');
  expect(screen.getByTestId('prj-rail-maps').getAttribute('aria-current')).toBeNull();

  fireEvent.click(screen.getByTestId('prj-rail-maps'));
  expect(screen.queryByTestId('ref-home-dropzone')).toBeNull();
});

test('FR-REF-06 the packs home lists cards sorted by name with image count, the first line of the notes and badges', async () => {
  setup({
    packs: [
      ...sampleRefSeed().packs,
      { name: 'Banners', pack_id: '01JMOCKPACKBANNERS00000000', images: [], badges: ['not_ready'] },
    ],
  });
  await waitFor(() => expect(cards()).toHaveLength(3));
  expect(cardNames()).toEqual(['armour', 'Banners', 'Weapons']);
  const w = card('Weapons');
  expect(w.dataset.recordId).toBe(WEAPONS);
  expect(within(w).getByTestId('ref-home-card-count').textContent).toBe('3 images');
  expect(within(w).getByTestId('ref-home-card-notes').textContent).toBe('Swords and axes');
  expect(within(card('Banners')).getByTestId('ref-home-card-badge').dataset.badge).toBe(
    'not_ready',
  );
  expect(core.names()).toContain('ref_list_packs');
});

test('FR-REF-13 a pack with local images shows its mosaic thumbnail; one without shows a placeholder', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  expect(within(card('Weapons')).getByTestId('ref-home-card-thumb').tagName).toBe('IMG');
  expect(within(card('armour')).getByTestId('ref-home-card-thumb').tagName).toBe('SPAN');
});

test('FR-REF-06 an empty library says how to start', async () => {
  setup({ packs: [] });
  expect((await screen.findByTestId('ref-home-empty')).textContent).toContain(
    'Create a pack, or drop a folder of reference images',
  );
});

test('FR-REF-07 + New pack creates a pack and opens it; a taken name is refused in the dialog', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  fireEvent.click(screen.getByTestId('ref-home-new'));
  await nameDialog('weapons');
  expect((await screen.findByTestId('ref-dialog-name-error')).textContent).toContain('taken');

  await nameDialog('Swords');
  await waitFor(() => expect(screen.getByTestId('ref-pack-name').textContent).toBe('Swords'));
  expect(screen.queryByTestId('ref-dialog-name')).toBeNull();
  expect(screen.getByTestId('ref-pack-empty')).toBeTruthy();
  expect(core.calls.filter((c) => c.cmd === 'ref_pack_create').map((c) => c.input)).toEqual([
    { name: 'weapons' },
    { name: 'Swords' },
  ]);
});

test('FR-REF-07 dropping a folder on the packs home creates one pack named after it, with (2) when taken', async () => {
  setup({
    dirs: {
      'D:/art/Weapons': ['D:/art/Weapons/axe.png'],
      'D:/art/Crypt moods': ['D:/art/Crypt moods/fog.png', 'D:/art/Crypt moods/sub/bones.webp'],
    },
  });
  await waitFor(() => expect(cards()).toHaveLength(2));
  await drop(['D:/art/Crypt moods']);
  await waitFor(() => expect(cardNames()).toContain('Crypt moods'));
  expect(screen.getByTestId('ref-add-summary-text').textContent).toBe('2 added');
  expect(within(card('Crypt moods')).getByTestId('ref-home-card-count').textContent).toBe(
    '2 images',
  );

  await drop(['D:/art/Weapons']);
  await waitFor(() => expect(cardNames()).toContain('Weapons (2)'));
  expect(core.calls.filter((c) => c.cmd === 'ref_pack_create').at(-1)?.input).toEqual({
    name: 'Weapons',
    paths: ['D:/art/Weapons'],
    suffix_if_taken: true,
  });
});

test('FR-REF-07 image files dropped on the packs home make no pack and say why', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  await drop(['D:/art/axe.png']);
  expect((await screen.findByTestId('ref-error')).dataset.code).toBe('drop_folder');
  expect(core.names()).not.toContain('ref_pack_create');
});

test('FR-REF-08 dropping images on the pack view copies them in; duplicates and unsupported files are reported and the batch continues', async () => {
  setup({
    sources: {
      'D:/art/again.png': { hash: 'swatch-a.png' },
      'D:/art/bad.png': { reason: 'corrupt' },
    },
  });
  await openWeapons();
  await drop([
    'D:/art/new.png',
    'D:/art/again.png',
    'D:/art/bad.png',
    'D:/art/notes.txt',
    'D:/art/z.webp',
  ]);
  await waitFor(() => expect(images()).toHaveLength(5));
  expect(images()).toEqual(['new.png', 'swatch-a.png', 'swatch-b.jpg', 'swatch-c.webp', 'z.webp']);
  expect(screen.getByTestId('ref-add-summary-text').textContent).toBe(
    '2 added · 1 already in pack · 2 skipped (corrupt, unsupported)',
  );
  const items = screen.getAllByTestId('ref-add-summary-item').map((i) => i.dataset.status);
  expect(items).toEqual(['already_in_pack', 'skipped', 'skipped']);
  expect(core.calls.find((c) => c.cmd === 'ref_pack_add_images')?.input).toEqual({
    pack_id: WEAPONS,
    paths: [
      'D:/art/new.png',
      'D:/art/again.png',
      'D:/art/bad.png',
      'D:/art/notes.txt',
      'D:/art/z.webp',
    ],
  });
});

test('FR-REF-08 a second drop while one copies waits its turn; the first keeps its progress', async () => {
  setup();
  await openWeapons();
  const h = core.handlersSnapshot();
  const add = h.ref_pack_add_images!;
  let release: () => void = () => undefined;
  let first = true;
  core.useHandlers({
    ...h,
    ref_pack_add_images: (input) => {
      if (!first) return add(input);
      first = false;
      return new Promise<void>((r) => (release = r)).then(() => add(input));
    },
  });
  await drop(['D:/art/a1.png', 'D:/art/a2.png', 'D:/art/a3.png']);
  await drop(['D:/art/b1.png']);
  await act(() => new Promise((r) => setTimeout(r, 30)));
  const adds = () => core.calls.filter((c) => c.cmd === 'ref_pack_add_images');
  expect(adds()).toHaveLength(1);
  act(() => core.emit('ref:add-progress', { done: 2, total: 3 }));
  expect(screen.getByTestId('ref-add-progress').textContent).toBe('Copying 2 / 3');
  await act(async () => release());
  await waitFor(() => expect(adds()).toHaveLength(2));
  expect(adds()[1]!.input.paths).toEqual(['D:/art/b1.png']);
  await waitFor(() => expect(images()).toContain('b1.png'));
  expect(images()).toContain('a3.png');
  expect(screen.queryByTestId('ref-add-progress')).toBeNull();
});

test('FR-REF-08 a taken file name gets (2)', async () => {
  setup();
  await openWeapons();
  await drop(['E:/other/swatch-a.png']);
  await waitFor(() => expect(images()).toContain('swatch-a (2).png'));
});

test('FR-REF-08 a pasted file list adds to the open pack; a text field keeps its paste', async () => {
  setup();
  await openWeapons();
  const paste = (target: EventTarget, text: string) => {
    const e = new Event('paste', { bubbles: true, cancelable: true }) as Event & {
      clipboardData: unknown;
    };
    e.clipboardData = { getData: () => text, items: [] };
    act(() => void target.dispatchEvent(e));
  };
  paste(screen.getByTestId('ref-pack-notes'), '"D:\\art\\pasted.png"');
  expect(core.names()).not.toContain('ref_pack_add_images');
  paste(document.body, '"D:\\art\\pasted.png"\r\n"D:\\art\\two.jpg"');
  await waitFor(() => expect(images()).toContain('pasted.png'));
  expect(images()).toContain('two.jpg');
});

test('FR-REF-08 Add… falls back to typed paths where no native dialog exists (#53)', async () => {
  setup();
  await openWeapons();
  fireEvent.click(screen.getByTestId('ref-pack-add'));
  const field = await screen.findByTestId('ref-pack-add-paths');
  fireEvent.change(field, { target: { value: 'D:/art/one.png\nD:/art/two.png' } });
  fireEvent.click(screen.getByTestId('ref-pack-add-confirm'));
  await waitFor(() => expect(images()).toHaveLength(5));
});

test('FR-REF-09 the pack view re-lists on window focus and on Refresh, so Explorer additions appear', async () => {
  setup();
  await openWeapons();
  mock.addUserFile(WEAPONS, { file: 'explorer.png' });
  act(() => void window.dispatchEvent(new Event('focus')));
  await waitFor(() => expect(images()).toContain('explorer.png'));
  mock.addUserFile(WEAPONS, { file: 'again.png' });
  fireEvent.click(screen.getByTestId('ref-pack-refresh'));
  await waitFor(() => expect(images()).toContain('again.png'));
});

test('FR-REF-14 a pack thumbnail that failed to load is tried again after a refresh gives a new one', async () => {
  setup();
  await openWeapons();
  const first = () => screen.getAllByTestId('ref-pack-image')[0]!;
  const img = first().querySelector('img')!;
  fireEvent.error(img);
  expect(first().querySelector('img')).toBeNull();
  const h = core.handlersSnapshot();
  const get = h.ref_pack_get!;
  core.useHandlers({
    ...h,
    ref_pack_get: async (input) => {
      const p = (await get(input)) as { images: { thumb_url?: string | null }[] };
      return {
        ...p,
        images: p.images.map((i) => ({ ...i, thumb_url: i.thumb_url && `${i.thumb_url}?v=2` })),
      };
    },
  });
  fireEvent.click(screen.getByTestId('ref-pack-refresh'));
  await waitFor(() => expect(first().querySelector('img')?.getAttribute('src')).toMatch(/\?v=2$/));
});

test('FR-REF-10 renaming from the pack view and from the card menu; a taken name is refused', async () => {
  setup();
  await openWeapons();
  fireEvent.click(screen.getByTestId('ref-pack-name'));
  await nameDialog('ARMOUR');
  expect((await screen.findByTestId('ref-dialog-name-error')).textContent).toContain('taken');
  await nameDialog('Blades');
  await waitFor(() => expect(screen.getByTestId('ref-pack-name').textContent).toBe('Blades'));
  expect(core.calls.filter((c) => c.cmd === 'ref_pack_rename').at(-1)?.input).toEqual({
    pack_id: WEAPONS,
    name: 'Blades',
  });

  fireEvent.click(screen.getByTestId('ref-pack-back'));
  await waitFor(() => expect(cardNames()).toEqual(['armour', 'Blades']));
  fireEvent.keyDown(within(card('armour')).getByRole('button'), { key: 'F2' });
  await nameDialog('Armour set');
  await waitFor(() => expect(cardNames()).toEqual(['Armour set', 'Blades']));
});

test('FR-REF-11 notes changed outside the app show after Refresh; a blur doesn’t write the old text back', async () => {
  setup();
  await openWeapons();
  const notes = () => screen.getByTestId('ref-pack-notes') as HTMLTextAreaElement;
  mock.packs.get(WEAPONS)!.notes = 'Edited in Explorer';
  fireEvent.click(screen.getByTestId('ref-pack-refresh'));
  await waitFor(() => expect(notes().value).toBe('Edited in Explorer'));
  fireEvent.focus(notes());
  fireEvent.blur(notes());
  await act(() => new Promise((r) => setTimeout(r, 50)));
  expect(core.calls.filter((c) => c.cmd === 'ref_pack_update')).toHaveLength(0);
  expect(mock.packs.get(WEAPONS)!.notes).toBe('Edited in Explorer');
});

test('FR-REF-11 notes and per-image roles are saved with ref_pack_update', async () => {
  setup();
  await openWeapons();
  const notes = screen.getByTestId('ref-pack-notes');
  fireEvent.change(notes, { target: { value: 'Steel only' } });
  fireEvent.blur(notes);
  await waitFor(() =>
    expect(core.calls.find((c) => c.cmd === 'ref_pack_update')?.input).toEqual({
      pack_id: WEAPONS,
      notes: 'Steel only',
    }),
  );

  const img = screen.getAllByTestId('ref-pack-image')[1]!;
  const role = within(img).getByTestId('ref-pack-image-role');
  expect(role.dataset.value).toBe('none');
  fireEvent.click(role);
  fireEvent.pointerDown(role, { button: 0, ctrlKey: false, pointerType: 'mouse' });
  const option = (await screen.findAllByTestId('ref-pack-image-role-option')).find(
    (o) => o.dataset.recordId === 'identity',
  )!;
  fireEvent.click(option);
  await waitFor(() =>
    expect(core.calls.filter((c) => c.cmd === 'ref_pack_update').at(-1)?.input).toEqual({
      pack_id: WEAPONS,
      roles: { 'swatch-b.jpg': 'identity' },
    }),
  );
  await waitFor(() =>
    expect(
      within(screen.getAllByTestId('ref-pack-image')[1]!).getByTestId('ref-pack-image-role').dataset
        .value,
    ).toBe('identity'),
  );
});

test('FR-REF-12 delete moves a pack to Recently deleted; Restore brings it back, with (2) when the name is taken', async () => {
  setup();
  await openWeapons();
  fireEvent.click(screen.getByTestId('ref-pack-delete'));
  fireEvent.click(await screen.findByTestId('ref-dialog-delete-confirm'));
  await waitFor(() => expect(cardNames()).toEqual(['armour']));
  const trash = screen.getByTestId('ref-trash-list');
  expect(within(trash).getByTestId('ref-trash-item').dataset.kind).toBe('pack');

  fireEvent.click(screen.getByTestId('ref-home-new'));
  await nameDialog('Weapons');
  await waitFor(() => expect(screen.getByTestId('ref-pack-name').textContent).toBe('Weapons'));
  fireEvent.click(screen.getByTestId('ref-pack-back'));
  await waitFor(() => expect(screen.getByTestId('ref-trash-list')).toBeTruthy());
  fireEvent.click(screen.getByTestId('ref-trash-restore'));
  await waitFor(() => expect(cardNames()).toEqual(['armour', 'Weapons', 'Weapons (2)']));
  expect(screen.queryByTestId('ref-trash-list')).toBeNull();
  expect(mock.packs.get(WEAPONS)?.name).toBe('Weapons (2)');
});

test('FR-REF-12 removing an image moves it to the References trash, restorable', async () => {
  setup();
  await openWeapons();
  const img = screen.getAllByTestId('ref-pack-image')[0]!;
  fireEvent.click(within(img).getByTestId('ref-pack-image-remove'));
  await waitFor(() => expect(images()).toEqual(['swatch-b.jpg', 'swatch-c.webp']));
  expect(mock.trash[0]).toMatchObject({ kind: 'images', files: ['swatch-a.png'] });
  fireEvent.click(screen.getByTestId('ref-pack-back'));
  fireEvent.click(await screen.findByTestId('ref-trash-restore'));
  await waitFor(() => expect(mock.packs.get(WEAPONS)?.images).toHaveLength(3));
});

test('FR-REF-14 pack view thumbnails load lazily; cloud-only and corrupt images are badged', async () => {
  setup({
    packs: [
      {
        name: 'Weapons',
        pack_id: WEAPONS,
        images: [
          { file: 'a.png' },
          { file: 'b.png', state: 'cloud_only' },
          { file: 'c.png', state: 'corrupt' },
        ],
      },
    ],
  });
  await waitFor(() => expect(cards()).toHaveLength(1));
  fireEvent.click(within(card('Weapons')).getByRole('button'));
  await waitFor(() => expect(images()).toHaveLength(3));
  const [a, b, c] = screen.getAllByTestId('ref-pack-image') as [
    HTMLElement,
    HTMLElement,
    HTMLElement,
  ];
  expect(a.querySelector('img')?.getAttribute('loading')).toBe('lazy');
  expect(within(b).getByTestId('ref-pack-image-badge').textContent).toBe('cloud');
  expect(within(c).getByTestId('ref-pack-image-badge').textContent).toBe('corrupt');
});

test('FR-REF-29 a failed command leaves the pack as it was and shows the error', async () => {
  setup();
  await openWeapons();
  const h = core.handlersSnapshot();
  core.useHandlers({
    ...h,
    ref_pack_remove_images: () => Promise.reject({ code: 'locked', message: 'The file is locked' }),
  });
  fireEvent.click(
    within(screen.getAllByTestId('ref-pack-image')[0]!).getByTestId('ref-pack-image-remove'),
  );
  expect((await screen.findByTestId('ref-error')).dataset.code).toBe('locked');
  expect(images()).toHaveLength(3);
});

test('FR-REF-04 a folder without pack.json is listed as not ready and cannot be opened', async () => {
  setup({
    packs: [
      ...sampleRefSeed().packs,
      { name: 'Loose', images: [{ file: 'x.png' }], notReady: true },
    ],
  });
  await waitFor(() => expect(cards()).toHaveLength(3));
  const loose = card('Loose');
  expect(loose.dataset.recordId).toBe('C:/pf-mock/library/References/Loose');
  expect(within(loose).getByTestId('ref-home-card-badge').dataset.badge).toBe('not_ready');
  const open = within(loose).getByRole('button') as HTMLButtonElement;
  expect(open.disabled).toBe(true);
  fireEvent.click(open);
  expect(core.names()).not.toContain('ref_pack_get');
});

test('BL-35 a pack card tooltip is relative to the library root, never the absolute path', async () => {
  setup({
    packs: [
      ...sampleRefSeed().packs,
      { name: 'Loose', images: [{ file: 'x.png' }], notReady: true },
    ],
  });
  await waitFor(() => expect(cards()).toHaveLength(3));
  for (const c of cards()) expect(c.title).not.toContain('C:/pf-mock');
  expect(card('Loose').title).toBe(
    'References/Loose (not ready: Pocket Forge couldn’t write pack.json)',
  );
  const name = within(cards()[0]!).getByTestId('ref-home-card-name').textContent;
  expect(cards()[0]!.title).toBe(`References/${name}`);
});

test('FR-REF-05 FR-REF-28 what a listing did is shown from the cards’ notices and can be dismissed', async () => {
  setup({
    packs: [
      {
        ...sampleRefSeed().packs[0]!,
        notices: [
          {
            code: 'conflict_resolved',
            message: 'Weapons had a sync conflict; the newer version was kept',
          },
        ],
      },
    ],
  });
  const notice = await screen.findByTestId('ref-home-notice');
  expect(notice.dataset.code).toBe('conflict_resolved');
  fireEvent.click(within(screen.getByTestId('ref-home-notices')).getByRole('button'));
  expect(screen.queryByTestId('ref-home-notices')).toBeNull();
});

test('R61 a trashed pack.json has no Restore and is labelled as such', async () => {
  setup({
    trash: [
      {
        id: '01JMOCKTRASHPACKJSON000000',
        kind: 'images',
        pack_id: '01JMOCKPACKWEAPONS00000000',
        pack_name: 'Weapons',
        files: ['pack.json'],
        trashed_at: '2026-11-08T12:00:00Z',
        purge_at: '2026-12-08T12:00:00Z',
        restorable: false,
      },
    ],
  });
  const row = await screen.findByTestId('ref-trash-item');
  expect(row.textContent).toContain('Weapons’s older pack.json');
  expect(row.textContent).not.toContain('1 image');
  expect(within(row).queryByTestId('ref-trash-restore')).toBeNull();
});

test('FR-REF-12 Recently deleted shows the pack name and the purge date', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  fireEvent.contextMenu(card('armour'));
  fireEvent.click(await screen.findByTestId('ref-home-card-delete'));
  fireEvent.click(await screen.findByTestId('ref-dialog-delete-confirm'));
  const row = await screen.findByTestId('ref-trash-item');
  expect(row.textContent).toContain('armour');
  // History's day format (walk R7), not the raw ISO date.
  const day = new Date('2026-12-08T12:00:00Z').toLocaleDateString([], {
    month: 'short',
    day: 'numeric',
  });
  expect(row.textContent).toContain(`until ${day}`);
  expect(row.textContent).not.toContain('2026-12-08');
  fireEvent.click(within(row).getByTestId('ref-trash-restore'));
  await waitFor(() => expect(cardNames()).toEqual(['armour', 'Weapons']));
  expect(core.calls.find((c) => c.cmd === 'ref_trash_restore')?.input).toEqual({
    id: expect.stringMatching(/^01JMOCKTRASH/),
  });
});

test('FR-REF-28 walk R5: a listing notice survives an overlapping listing that lands last', async () => {
  setup({
    packs: [
      {
        ...sampleRefSeed().packs[0]!,
        notices: [{ code: 'conflict_resolved', message: 'Weapons had a sync conflict' }],
      },
    ],
  });
  // The first listing (with the notice) answers after the second (without it).
  const h = core.handlersSnapshot();
  let n = 0;
  core.useHandlers({
    ...h,
    ref_list_packs: async (i) => {
      const first = n++ === 0;
      const out = await h.ref_list_packs!(i);
      if (first) await new Promise((r) => setTimeout(r, 60));
      return out;
    },
  });
  await act(async () => {
    void refs.refreshList();
    await refs.refreshList();
  });
  await waitFor(() => expect(n).toBe(2));
  const notice = await screen.findByTestId('ref-home-notice');
  expect(notice.dataset.code).toBe('conflict_resolved');
  fireEvent.click(within(screen.getByTestId('ref-home-notices')).getByRole('button'));
  expect(screen.queryByTestId('ref-home-notices')).toBeNull();
});

test('FR-REF-28 walk R5: what a listing moves to the trash is listed by the same refresh', async () => {
  setup();
  await waitFor(() => expect(cards()).toHaveLength(2));
  const h = core.handlersSnapshot();
  core.useHandlers({
    ...h,
    // A listing that resolves a conflict: the loser lands in the trash before it answers.
    ref_list_packs: async (i) => {
      await new Promise((r) => setTimeout(r, 30));
      mock.trash.unshift({
        id: '01JMOCKTRASHLOSER000000000',
        kind: 'images',
        pack_id: '01JMOCKPACKWEAPONS00000000',
        pack_name: 'Weapons',
        files: ['pack-LAPTOP.json'],
        trashed_at: '2026-11-08T12:00:00Z',
        purge_at: '2026-12-08T12:00:00Z',
        restorable: false,
      });
      return h.ref_list_packs!(i);
    },
  });
  await act(() => refs.refresh());
  expect(screen.getAllByTestId('ref-trash-item').map((r) => r.dataset.recordId)).toContain(
    '01JMOCKTRASHLOSER000000000',
  );
});
