// The References block inside RGN's Generate panel (reference-packs §7, R44): mounted between
// Model and Prompt, the selection saved with the draft. (Submit blocked under a slot's min needs
// an EDT region: e2e/ref-block.spec.ts.)
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { Tooltip } from 'radix-ui';

import refsCaps from '../../../../fixtures/rgn/capabilities-refs.json';
import type { Capabilities } from '../../api';
import { refMock, sampleRefSeed, type RefMockSlot } from '../../api/mocks/ref';
import { installRgnMock } from '../../api/mocks/rgn';
import { baseSnapshot, installFakeTauri, type FakeTauri } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { regionEditFeature } from '../region-edit';
import { block } from './blockStore';

// The first test loads the whole Generate panel (Radix, EDT, RGN): slow on a cold, busy runner.
vi.setConfig({ testTimeout: 15_000 });

const GeneratePanel = regionEditFeature.slots.right!;
const REFS = refsCaps as Capabilities;
const REQUIRED: Capabilities = {
  ...REFS,
  model_id: 'fake-required',
  slots: [
    {
      name: 'characterImages',
      role: 'identity',
      min: 1,
      max: 1,
      cost_affecting: false,
      array: true,
    },
  ],
};
const NONE: Capabilities = { ...REFS, model_id: 'fake-none', slots: [] };
const SLOTS: Record<string, RefMockSlot[]> = {
  'fake-refs': [
    { name: 'styleImages', role: 'style', min: 0, max: 2 },
    { name: 'characterImages', role: 'identity', min: 0, max: 1 },
  ],
  'fake-required': [{ name: 'characterImages', role: 'identity', min: 1, max: 1 }],
};

let fake: FakeTauri;

function polyfill(): void {
  const g = globalThis as unknown as Record<string, unknown>;
  g.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => undefined;
  proto.releasePointerCapture ??= () => undefined;
  proto.scrollIntoView ??= () => undefined;
}

async function setup(first: 'fake-refs' | 'fake-required' = 'fake-refs', projectRefs = 2) {
  polyfill();
  fake = installFakeTauri(baseSnapshot());
  const w = window as unknown as {
    __pfMockHandlers: Record<string, (i: unknown) => unknown>;
    __pfMockSnapshot: Record<string, unknown>;
  };
  const models = [
    { id: 'fake-refs', name: 'Refs' },
    { id: 'fake-required', name: 'Required' },
    { id: 'fake-none', name: 'None' },
  ];
  installRgnMock({
    image: { w: 2048, h: 2560 },
    models: first === 'fake-refs' ? models : [models[1]!, models[0]!, models[2]!],
    capabilities: { 'fake-refs': REFS, 'fake-required': REQUIRED, 'fake-none': NONE },
    spend: { cap: 100, threshold: 10 },
  });
  const refs = refMock(
    {
      ...sampleRefSeed(),
      slots: SLOTS,
      projectRefs: ['a.png', 'b.png', 'c.png'].slice(0, projectRefs).map((file) => ({ file })),
    },
    { emit: fake.emit },
  );
  Object.assign(w.__pfMockHandlers, refs.handlers, {
    project_open: () => ({ ...baseSnapshot(), ...w.__pfMockSnapshot }),
    provider_status: () => ({
      mode: 'fake',
      configured: true,
      connectivity: 'ok',
      store_unavailable: false,
    }),
  });
  projectStore.reset();
  block.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(screen.getByTestId('ref-block')).toBeTruthy());
  await waitFor(() => expect(screen.getAllByTestId('ref-block-item').length).toBe(projectRefs));
}

beforeEach(() => projectStore.reset());
afterEach(() => {
  // Close while mounted, so RGN's session detaches (its store is module state).
  act(() => projectStore.reset());
  cleanup();
  fake?.uninstall();
});

test('FR-REF-15 the block sits between Model and Prompt and hides for a model without slots', async () => {
  await setup();
  const order = [...document.querySelectorAll('[data-testid]')]
    .map((e) => e.getAttribute('data-testid'))
    .filter((t) => ['rgn-panel-model', 'ref-block', 'prm-block-prompt'].includes(t!));
  expect(order).toEqual(['rgn-panel-model', 'ref-block', 'prm-block-prompt']);

  fireEvent.pointerDown(screen.getByTestId('rgn-panel-model'), {
    button: 0,
    ctrlKey: false,
    pointerType: 'mouse',
  });
  const list = await screen.findByRole('listbox');
  fireEvent.click(within(list).getByText('None'));
  await waitFor(() => expect(screen.queryByTestId('ref-block')).toBeNull());
});

test('FR-REF-20 a tick is written with the draft through RGN’s debounced draft save', async () => {
  await setup();
  const first = screen.getAllByTestId('ref-block-item')[0]!;
  fireEvent.click(within(first).getByTestId('ref-block-item-check'));
  await waitFor(
    () =>
      expect(
        fake.callsOf('project_put_draft').some((c) => {
          const d = (c.draft as { references?: { items: { key: string; on: boolean }[] } })
            .references;
          return d?.items.some((i) => i.key === 'project:a.png' && !i.on);
        }),
      ).toBe(true),
    { timeout: 3000 },
  );
  const draft = fake.callsOf('project_put_draft').at(-1)!.draft as Record<string, unknown>;
  // R53: only roles the user chose are saved; the defaults (style here) are re-derived per model.
  expect(draft.references).toEqual({
    packs: [],
    items: [
      { key: 'project:a.png', on: false, role: null },
      { key: 'project:b.png', on: true, role: null },
    ],
  });
  // The rest of the draft is still RGN's.
  expect(draft).toHaveProperty('final_prompt');
  expect(draft).toHaveProperty('model_id', 'fake-refs');
});
