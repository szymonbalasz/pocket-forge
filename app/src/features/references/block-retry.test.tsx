// REF slice 6 (RGN integration) on the References block: the Retry reset (FR-REF-26,
// FR-RGN-44), the selection RGN sends (FR-RGN-41) and the cost key that re-estimates (FR-RGN-42,
// FR-REF-24). Over api/mocks/ref.ts, whose `__ref_retry` hook mirrors pf-core `references::retry`.
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';

import type { DraftReferences } from '../../api';
import { refMock, sampleRefSeed, type RefMockRow, type RefMockSeed } from '../../api/mocks/ref';
import { installTestCore } from '../../api/mocks/testCore';
import { block, referencesCostKey, referencesDraft, referencesForSend } from './blockStore';
import { ReferencesBlock } from './ReferencesBlock';

const core = installTestCore();
const WEAPONS = '01JMOCKPACKWEAPONS00000000';
// Like fixtures/rgn/capabilities-refs.json: style ≤ 2 cost-affecting, identity ≤ 1.
const SLOTS: RefMockSeed['slots'] = {
  'fake-refs': [
    { name: 'styleImages', role: 'style', min: 0, max: 2, cost_affecting: true },
    { name: 'characterImages', role: 'identity', min: 0, max: 1 },
  ],
};
const onChange = vi.fn<(save: boolean) => void>();

function setup() {
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.scrollIntoView ??= () => undefined;
  const m = refMock(
    {
      ...sampleRefSeed(),
      slots: SLOTS,
      projectRefs: [{ file: 'mood-a.png' }, { file: 'mood-b.png' }, { file: 'mood-c.png' }],
    },
    { emit: core.emit },
  );
  core.useHandlers(m.handlers);
  render(
    <ReferencesBlock
      projectId="01JPROJECT0000000000000000"
      draftReferences={null}
      modelId="fake-refs"
      hasSlots
      readOnly={false}
      onChange={onChange}
    />,
  );
  return m;
}

const items = () => screen.queryAllByTestId('ref-block-item');
const item = (key: string) => items().find((i) => i.dataset.recordId === key)!;
const on = () =>
  items()
    .filter((i) => i.dataset.on === 'true')
    .map((i) => i.dataset.recordId);
const badges = (key: string) =>
  within(item(key))
    .queryAllByTestId('ref-block-item-badge')
    .map((b) => [b.dataset.badge, b.textContent]);
const idle = () => waitFor(() => expect(block.get().loading).toBe(false));

/** A job's `references[]` as the RGN mock records it (schema §4.6). */
const row = (path: string, hash: string, patch: Partial<RefMockRow> = {}): RefMockRow => ({
  source: path.startsWith('references/') ? 'project' : 'pack',
  pack_id: path.startsWith('references/') ? null : WEAPONS,
  pack_name: path.startsWith('references/') ? null : 'Weapons',
  path,
  content_hash: hash,
  slot: 'styleImages',
  role: 'style',
  cost_affecting: true,
  ...patch,
});

beforeEach(() => {
  block.reset();
  onChange.mockClear();
});
afterEach(() => cleanup());

test('FR-REF-26 FR-RGN-44 Retry restores the job’s ticks; changed since NNN, missing, the rest off', async () => {
  const m = setup();
  await waitFor(() => expect(items().length).toBe(3));
  // The job sent mood-b and Weapons' swatch-a; mood-a was repainted since, gone.png was deleted.
  m.state.projectRefs[1]!.hash = 'mood-b v2';
  const draft = (await m.handlers.__ref_retry!({
    rows: [
      row('references/mood-b.png', 'mood-b.png'),
      row('References/Weapons/swatch-a.png', 'swatch-a.png'),
      row('references/gone.png', 'gone.png'),
    ],
    label: '001',
    has_slots: true,
  })) as DraftReferences;
  onChange.mockClear();
  act(() => block.restore(draft));
  await idle();
  const a = `pack:${WEAPONS}:swatch-a.png`;
  await waitFor(() => expect(item(a)).toBeTruthy());
  expect(on()).toEqual(['project:mood-b.png', a]);
  expect(item('project:mood-a.png').dataset.on).toBe('false');
  expect(item(`pack:${WEAPONS}:swatch-b.jpg`).dataset.on).toBe('false');
  expect(badges('project:mood-b.png')).toEqual([['changed_since:001', 'changed since 001']]);
  expect(badges(a)).toEqual([]);
  expect(item('project:gone.png').dataset.on).toBe('false');
  expect(badges('project:gone.png')).toEqual([['missing', 'missing']]);
  // Saved with the draft (FR-REF-20), flags included so a reopen shows them again.
  expect(onChange).toHaveBeenCalledWith(true);
  const saved = referencesDraft().references!;
  expect(saved.packs).toEqual([{ pack_id: WEAPONS, name: 'Weapons' }]);
  expect(saved.items.find((i) => i.key === 'project:mood-b.png')).toMatchObject({
    on: true,
    retry_of: '001',
    changed: true,
  });
});

test('FR-REF-26 after a submit the Retry flags are cleared and missing items leave', async () => {
  const m = setup();
  await waitFor(() => expect(items().length).toBe(3));
  m.state.projectRefs[0]!.hash = 'mood-a v2';
  const draft = (await m.handlers.__ref_retry!({
    rows: [row('references/mood-a.png', 'mood-a.png'), row('references/gone.png', 'gone.png')],
    label: '002',
    has_slots: true,
  })) as DraftReferences;
  act(() => block.restore(draft));
  await idle();
  await waitFor(() => expect(badges('project:mood-a.png')).toHaveLength(1));
  act(() => block.settle());
  await idle();
  await waitFor(() => expect(badges('project:mood-a.png')).toEqual([]));
  expect(items().map((i) => i.dataset.recordId)).not.toContain('project:gone.png');
  expect(on()).toEqual(['project:mood-a.png']);
  const saved = referencesDraft().references!;
  expect(saved.items.every((i) => i.retry_of === undefined && i.changed === undefined)).toBe(true);
});

test('FR-RGN-41 BL-34 estimate and submit send the selection the block shows, the defaults included', async () => {
  setup();
  await waitFor(() => expect(items().length).toBe(3));
  await idle();
  // REF's defaults (no saved selection): the resolved ticks are sent, so a default-ticked file
  // that vanishes is refused by the core instead of re-derived without it.
  const defaults = referencesForSend();
  expect(defaults).not.toBeNull();
  expect(defaults!.items.filter((i) => i.on)).toHaveLength(2);
  expect(defaults!.items.filter((i) => i.on).map((i) => i.key)).toEqual(
    items()
      .filter((e) => e.dataset.on === 'true')
      .map((e) => e.dataset.recordId),
  );
  const sel: DraftReferences = {
    packs: [],
    items: [{ key: 'project:mood-a.png', on: false, role: 'style' }],
  };
  act(() => block.restore(sel));
  await idle();
  expect(referencesForSend()?.items.find((i) => i.key === 'project:mood-a.png')?.on).toBe(false);
});

test('FR-RGN-42 FR-REF-24 only ticks in cost-affecting slots change the cost key', async () => {
  setup();
  await waitFor(() => expect(items().length).toBe(3));
  await idle();
  const base = referencesCostKey();
  expect(base).toBe('styleImages=project:mood-a.png,styleImages=project:mood-b.png');
  // mood-c → identity (not cost-affecting) and ticked: the key is unchanged.
  act(() => block.setRole('project:mood-c.png', 'identity'));
  await idle();
  act(() => block.toggle('project:mood-c.png'));
  await idle();
  expect(item('project:mood-c.png').dataset.on).toBe('true');
  expect(referencesCostKey()).toBe(base);
  // Unticking a style item changes it.
  act(() => block.toggle('project:mood-a.png'));
  await idle();
  expect(referencesCostKey()).toBe('styleImages=project:mood-b.png');
});

test('FR-REF-26 Appendix A step 9: a deleted pack’s items are missing; restored, they resolve again', async () => {
  const m = setup();
  await waitFor(() => expect(items().length).toBe(3));
  const draft = (await m.handlers.__ref_retry!({
    rows: [row('References/Weapons/swatch-a.png', 'swatch-a.png')],
    label: '001',
    has_slots: true,
  })) as DraftReferences;
  act(() => block.restore(draft));
  await idle();
  const a = `pack:${WEAPONS}:swatch-a.png`;
  await waitFor(() => expect(item(a)?.dataset.on).toBe('true'));
  // Deleted in the packs home: the core emits `ref:packs-changed`; the block resolves again.
  const { item_id } = (await m.handlers.ref_pack_delete!({ pack_id: WEAPONS })) as {
    item_id: string;
  };
  await waitFor(() => expect(badges(a)).toEqual([['missing', 'missing']]));
  expect(item(a).dataset.on).toBe('false');
  await m.handlers.ref_trash_restore!({ id: item_id });
  await waitFor(() => expect(badges(a)).toEqual([]));
});
