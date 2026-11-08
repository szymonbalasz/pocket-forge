// REF References block on a mocked `ref_resolve` (api/mocks/ref.ts), over
// fixtures/rgn/capabilities-refs.json (style ≤ 2 cost-affecting, identity ≤ 1).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';

import refsCaps from '../../../../fixtures/rgn/capabilities-refs.json';
import type { DraftReferences } from '../../api';
import { refMock, sampleRefSeed, type RefMockSeed } from '../../api/mocks/ref';
import { installTestCore } from '../../api/mocks/testCore';
import { block, referencesBlocker, referencesDraft } from './blockStore';
import { ReferencesBlock } from './ReferencesBlock';

const core = installTestCore();
const WEAPONS = '01JMOCKPACKWEAPONS00000000';
const ARMOUR = '01JMOCKPACKARMOUR000000000';
const SLOTS: RefMockSeed['slots'] = {
  'fake-refs': refsCaps.slots.map(({ name, role, min, max }) => ({
    name,
    role: role as 'style' | 'identity',
    min,
    max,
  })),
  'fake-one': [{ name: 'reference', role: null, min: 0, max: 4 }],
  'fake-one-style': [{ name: 'styleImages', role: 'style', min: 0, max: 3 }],
  'fake-two': [{ name: 'reference', role: null, min: 0, max: 2 }],
  'fake-required': [{ name: 'characterImages', role: 'identity', min: 1, max: 1 }],
};

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

const onChange = vi.fn<(save: boolean) => void>();

interface Props {
  modelId: string | null;
  hasSlots?: boolean;
  readOnly?: boolean;
  draft?: DraftReferences | null;
}

function setup(props: Props, patch: Partial<RefMockSeed> = {}) {
  const m = refMock(
    {
      ...sampleRefSeed(),
      slots: SLOTS,
      projectRefs: [{ file: 'mood-b.png' }, { file: 'Mood-a.jpg' }],
      ...patch,
    },
    { emit: core.emit },
  );
  core.useHandlers(m.handlers);
  const el = (p: Props) => (
    <ReferencesBlock
      projectId="01JPROJECT0000000000000000"
      draftReferences={p.draft ?? null}
      modelId={p.modelId}
      hasSlots={p.hasSlots ?? true}
      readOnly={p.readOnly ?? false}
      onChange={onChange}
    />
  );
  const view = render(el(props));
  return { ...view, rerender: (p: Props) => view.rerender(el(p)), state: m.state };
}

const items = () => screen.queryAllByTestId('ref-block-item');
const keys = () => items().map((i) => i.dataset.recordId);
const item = (key: string) => items().find((i) => i.dataset.recordId === key)!;
const on = () =>
  items()
    .filter((i) => i.dataset.on === 'true')
    .map((i) => i.dataset.recordId);
const check = (key: string) =>
  within(item(key)).getByTestId('ref-block-item-check') as HTMLInputElement;
const resolves = () => core.calls.filter((c) => c.cmd === 'ref_resolve').map((c) => c.input);

async function pick(packId: string) {
  if (!screen.queryByTestId('ref-block-pack-list'))
    fireEvent.click(screen.getByTestId('ref-block-pack-picker'));
  const option = await waitFor(() => {
    const o = screen
      .getAllByTestId('ref-block-pack-option')
      .find((x) => x.dataset.recordId === packId);
    expect(o).toBeTruthy();
    return o!;
  });
  fireEvent.click(within(option).getByRole('checkbox'));
  await waitFor(() =>
    expect(
      screen.getAllByTestId('ref-block-group').some((g) => g.dataset.recordId === packId),
    ).toBe(true),
  );
  await waitFor(() => expect(block.get().loading).toBe(false));
}

beforeEach(() => {
  polyfill();
  block.reset();
  onChange.mockClear();
});
afterEach(cleanup);

test('FR-REF-15 the block is hidden and nothing is resolved when the model has no reference slots', async () => {
  const v = setup({ modelId: 'fake-none', hasSlots: false });
  expect(screen.queryByTestId('ref-block')).toBeNull();
  expect(resolves()).toHaveLength(0);
  v.rerender({ modelId: null });
  expect(screen.queryByTestId('ref-block')).toBeNull();
  v.rerender({ modelId: 'fake-one', hasSlots: true });
  await waitFor(() => expect(screen.getByTestId('ref-block')).toBeTruthy());
  expect(resolves()).toEqual([{ model_id: 'fake-one', references: null }]);
});

test('FR-REF-16 available items: the project references by name, then each picked pack in pick order', async () => {
  setup({ modelId: 'fake-one' });
  await waitFor(() => expect(items()).toHaveLength(2));
  expect(keys()).toEqual(['project:Mood-a.jpg', 'project:mood-b.png']);
  expect(screen.getAllByTestId('ref-block-group').map((g) => g.dataset.recordId)).toEqual([
    'project',
  ]);
  await pick(ARMOUR);
  await pick(WEAPONS);
  expect(screen.getAllByTestId('ref-block-group').map((g) => g.dataset.recordId)).toEqual([
    'project',
    ARMOUR,
    WEAPONS,
  ]);
  expect(keys()).toEqual([
    'project:Mood-a.jpg',
    'project:mood-b.png',
    `pack:${WEAPONS}:swatch-a.png`,
    `pack:${WEAPONS}:swatch-b.jpg`,
    `pack:${WEAPONS}:swatch-c.webp`,
  ]);
  expect(resolves().at(-1)?.references).toMatchObject({
    packs: [
      { pack_id: ARMOUR, name: 'armour' },
      { pack_id: WEAPONS, name: 'Weapons' },
    ],
  });
});

test('FR-REF-17 project references are ticked by default and untickable; earlier ticks are kept', async () => {
  setup({ modelId: 'fake-one' });
  await waitFor(() => expect(on()).toEqual(['project:Mood-a.jpg', 'project:mood-b.png']));
  fireEvent.click(check('project:Mood-a.jpg'));
  await waitFor(() => expect(on()).toEqual(['project:mood-b.png']));
  await pick(WEAPONS);
  // New pack items are on while the slot has room; the unticked project item stays off.
  expect(on()).toEqual([
    'project:mood-b.png',
    `pack:${WEAPONS}:swatch-a.png`,
    `pack:${WEAPONS}:swatch-b.jpg`,
    `pack:${WEAPONS}:swatch-c.webp`,
  ]);
  expect(
    within(item(`pack:${WEAPONS}:swatch-c.webp`)).getByTestId('ref-block-item-badge').dataset.badge,
  ).toBe('cloud');
});

test('FR-REF-18 over the limit the first N are ticked, the rest are refused with "limit N reached"', async () => {
  setup({ modelId: 'fake-two' });
  await waitFor(() => expect(items()).toHaveLength(2));
  await pick(WEAPONS);
  expect(on()).toEqual(['project:Mood-a.jpg', 'project:mood-b.png']);
  expect(screen.getByTestId('ref-block-count').textContent).toBe('2 / 2');
  expect(screen.getByTestId('ref-block-over').textContent).toBe('5 available · 2 sent');
  const extra = check(`pack:${WEAPONS}:swatch-a.png`);
  expect(extra.disabled).toBe(true);
  expect(
    within(item(`pack:${WEAPONS}:swatch-a.png`)).getByTestId('ref-block-item-limit').textContent,
  ).toBe('limit 2 reached');

  // Swap one: untick a project item, then the pack item can be ticked.
  fireEvent.click(check('project:mood-b.png'));
  await waitFor(() => expect(check(`pack:${WEAPONS}:swatch-a.png`).disabled).toBe(false));
  fireEvent.click(check(`pack:${WEAPONS}:swatch-a.png`));
  await waitFor(() => expect(on()).toEqual(['project:Mood-a.jpg', `pack:${WEAPONS}:swatch-a.png`]));
});

test('FR-REF-18 a model change that lowers the limit turns off the last items, with a notice', async () => {
  const v = setup({ modelId: 'fake-one' });
  await waitFor(() => expect(items()).toHaveLength(2));
  await pick(WEAPONS);
  expect(on()).toHaveLength(4);
  v.rerender({ modelId: 'fake-two' });
  await waitFor(() => expect(on()).toEqual(['project:Mood-a.jpg', 'project:mood-b.png']));
  expect(screen.getAllByTestId('ref-block-notice')[0]!.dataset.code).toBe('limit_lowered');
});

test('FR-REF-18 a slot under its min is named in red and blocks submit (references_required)', async () => {
  setup({ modelId: 'fake-required' }, { projectRefs: [] });
  await waitFor(() => expect(screen.getByTestId('ref-block-under')).toBeTruthy());
  expect(screen.getByTestId('ref-block-under').textContent).toBe('Identity needs at least 1');
  expect(referencesBlocker()).toBe('References: Identity needs at least 1.');
  await pick(WEAPONS);
  await waitFor(() => expect(screen.queryByTestId('ref-block-under')).toBeNull());
  expect(referencesBlocker()).toBeNull();
});

test('FR-REF-19 roles: none for one slot without a role, a label for one slot with a role, a select per item for several', async () => {
  const v = setup({ modelId: 'fake-one' });
  await waitFor(() => expect(items()).toHaveLength(2));
  expect(screen.queryByTestId('ref-block-role')).toBeNull();
  expect(screen.queryAllByTestId('ref-block-item-role')).toHaveLength(0);

  v.rerender({ modelId: 'fake-one-style' });
  await waitFor(() => expect(screen.getByTestId('ref-block-role').textContent).toBe('Style'));
  expect(screen.queryAllByTestId('ref-block-item-role')).toHaveLength(0);

  v.rerender({ modelId: 'fake-refs' });
  await waitFor(() => expect(screen.getAllByTestId('ref-block-item-role')).toHaveLength(2));
  expect(screen.queryByTestId('ref-block-role')).toBeNull();
  expect(screen.getByTestId('ref-block-count').textContent).toBe('Style 2/2 · Identity 0/1');
});

test('FR-REF-17 FR-REF-19 a pack image defaults to its pack.json role when the model has that slot; a role change moves it', async () => {
  setup(
    { modelId: 'fake-refs' },
    {
      projectRefs: [],
      packs: [
        {
          name: 'Faces',
          pack_id: WEAPONS,
          images: [{ file: 'face.png', role: 'identity' }, { file: 'look.png' }],
        },
      ],
    },
  );
  await waitFor(() => expect(screen.getByTestId('ref-block-empty')).toBeTruthy());
  await pick(WEAPONS);
  const role = (k: string) => within(item(k)).getByTestId('ref-block-item-role').dataset.value;
  expect(role(`pack:${WEAPONS}:face.png`)).toBe('identity');
  expect(role(`pack:${WEAPONS}:look.png`)).toBe('style');
  expect(screen.getByTestId('ref-block-count').textContent).toBe('Style 1/2 · Identity 1/1');

  const trigger = within(item(`pack:${WEAPONS}:look.png`)).getByTestId('ref-block-item-role');
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: 'mouse' });
  const identity = (await screen.findAllByTestId('ref-block-item-role-option')).find(
    (o) => o.dataset.recordId === 'identity',
  )!;
  // The identity slot is full: a ticked item can't move into it.
  expect(identity.getAttribute('data-disabled')).not.toBeNull();
  const style = (await screen.findAllByTestId('ref-block-item-role-option')).find(
    (o) => o.dataset.recordId === 'style',
  )!;
  fireEvent.click(style);
});

test('FR-REF-20 ticks, picks and role changes are handed to the draft save; the draft selection is restored', async () => {
  const draft: DraftReferences = {
    packs: [{ pack_id: WEAPONS, name: 'Weapons' }],
    items: [{ key: 'project:Mood-a.jpg', on: false, role: null }],
  };
  setup({ modelId: 'fake-one', draft });
  await waitFor(() => expect(items()).toHaveLength(5));
  expect(resolves()[0]).toEqual({ model_id: 'fake-one', references: draft });
  expect(check('project:Mood-a.jpg').checked).toBe(false);
  expect(onChange).not.toHaveBeenCalledWith(true);

  act(() => fireEvent.click(check('project:Mood-a.jpg')));
  expect(onChange).toHaveBeenCalledWith(true);
  await waitFor(() => expect(check('project:Mood-a.jpg').checked).toBe(true));
  expect(referencesDraft().references?.items[0]).toEqual({
    key: 'project:Mood-a.jpg',
    on: true,
    role: null,
  });

  fireEvent.click(screen.getAllByTestId('ref-block-pack-unpick')[0]!);
  await waitFor(() => expect(items()).toHaveLength(2));
  expect(referencesDraft().references?.packs).toEqual([]);
});

test('FR-REF-20 with no draft selection and no edit, the draft field is left out', async () => {
  setup({ modelId: 'fake-one' });
  await waitFor(() => expect(items()).toHaveLength(2));
  expect(referencesDraft()).toEqual({});
});

test('FR-REF-27 a read-only project shows the block disabled', async () => {
  setup({ modelId: 'fake-one', readOnly: true });
  await waitFor(() => expect(items()).toHaveLength(2));
  expect(screen.getByTestId('ref-block').getAttribute('aria-disabled')).toBe('true');
  expect(check('project:Mood-a.jpg').disabled).toBe(true);
  expect((screen.getByTestId('ref-block-pack-picker') as HTMLButtonElement).disabled).toBe(true);
});

test('FR-REF-04 a not_ready pack cannot be picked', async () => {
  setup(
    { modelId: 'fake-one' },
    {
      packs: [
        { name: 'Locked', pack_id: ARMOUR, images: [{ file: 'x.png' }], badges: ['not_ready'] },
      ],
    },
  );
  await waitFor(() => expect(items()).toHaveLength(2));
  fireEvent.click(screen.getByTestId('ref-block-pack-picker'));
  const option = await screen.findByTestId('ref-block-pack-option');
  expect(option.dataset.disabled).toBe('true');
  expect((within(option).getByRole('checkbox') as HTMLInputElement).disabled).toBe(true);
});

test('FR-REF-15 the empty state says where references come from; a resolve error is shown, not hidden', async () => {
  setup({ modelId: 'fake-one' }, { projectRefs: [] });
  expect((await screen.findByTestId('ref-block-empty')).textContent).toContain(
    'No references. Add files to this project’s references folder or pick a pack',
  );
  cleanup();
  block.reset();
  setup({ modelId: 'fake-unknown' });
  expect((await screen.findByTestId('ref-block-error')).dataset.code).toBe('unknown_model');
});

test('FR-REF-26 a picked pack that is gone keeps its items as missing, unticked, with a notice', async () => {
  const draft: DraftReferences = {
    packs: [{ pack_id: '01JMOCKPACKGONE00000000000', name: 'Gone' }],
    items: [{ key: 'pack:01JMOCKPACKGONE00000000000:old.png', on: true, role: null }],
  };
  setup({ modelId: 'fake-one', draft });
  await waitFor(() => expect(items()).toHaveLength(3));
  const gone = item('pack:01JMOCKPACKGONE00000000000:old.png');
  expect(gone.dataset.on).toBe('false');
  expect(within(gone).getByTestId('ref-block-item-badge').dataset.badge).toBe('missing');
  expect(check('pack:01JMOCKPACKGONE00000000000:old.png').disabled).toBe(true);
  expect(screen.getAllByTestId('ref-block-notice').map((n) => n.dataset.code)).toContain(
    'pack_unavailable',
  );
  // The missing item stays in the draft, so a restored pack resolves again.
  expect(referencesDraft().references?.items.map((i) => i.key)).toContain(
    'pack:01JMOCKPACKGONE00000000000:old.png',
  );
});

test('FR-REF-27 the core’s read_only flag disables the block too', async () => {
  setup({ modelId: 'fake-one' }, { readOnly: true });
  await waitFor(() => expect(items()).toHaveLength(2));
  expect(check('project:Mood-a.jpg').disabled).toBe(true);
});
