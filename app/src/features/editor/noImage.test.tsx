// FR-GEN-03 (GEN slice 2): a project made by Generate new has no image until its first pick: the
// canvas says so, the tools are disabled with the reason `no_image`, and Export is disabled.
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { Tooltip } from 'radix-ui';
import { afterEach, beforeEach, expect, test } from 'vitest';

import type { ProjectSnapshot } from '../../api';
import { baseSnapshot, installFakeTauri } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { exportFeature } from '../export';
import { EditorPanel } from './EditorPanel';
import { EditorToolbar } from './EditorToolbar';
import { undoStack } from './session';

const ExportPanel = exportFeature.slots.right!;

function noImageSnapshot(): ProjectSnapshot {
  return {
    ...baseSnapshot(),
    image_path: null,
    original_image_id: null,
    has_image: false,
    width: 0,
    height: 0,
    format: null,
    layers: [],
  };
}

async function open(snap: ProjectSnapshot) {
  const fake = installFakeTauri(snap);
  const w = window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> };
  w.__pfMockHandlers.project_open = () => snap;
  await act(() => projectStore.open(snap.project_dir));
  return fake;
}

(globalThis as unknown as Record<string, unknown>).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

beforeEach(() => projectStore.reset());
afterEach(cleanup);

test('FR-GEN-03 the canvas of a project with no image says "No image yet" and draws nothing', async () => {
  const fake = await open(noImageSnapshot());
  render(<EditorPanel />);
  expect(screen.getByTestId('edt-canvas-no-image').textContent).toBe(
    'No image yet. Generate one in New image.',
  );
  expect(screen.queryByTestId('edt-canvas-stage')).toBeNull();
  // No tiles are asked for a 0 × 0 image.
  expect(fake.calls.some((c) => c.cmd === 'edt_tiles_open')).toBe(false);
});

test('FR-GEN-03 the tools are disabled with the reason no_image', async () => {
  await open(noImageSnapshot());
  render(
    <Tooltip.Provider>
      <EditorToolbar />
    </Tooltip.Provider>,
  );
  for (const t of ['pan', 'region', 'brush', 'eraser']) {
    const b = screen.getByTestId(`edt-toolbar-${t}`) as HTMLButtonElement;
    expect(b.disabled).toBe(true);
    expect(b.dataset.reason).toBe('no_image');
  }
});

test('walk L1 FR-GEN-29 "No image yet" has Undo and Redo (an undone first pick redoes from here)', async () => {
  await open(noImageSnapshot());
  render(
    <Tooltip.Provider>
      <EditorPanel />
      <EditorToolbar />
    </Tooltip.Provider>,
  );
  const redo = screen.getByTestId('edt-toolbar-redo') as HTMLButtonElement;
  expect(redo.disabled).toBe(true);
  const log: string[] = [];
  act(() =>
    undoStack.push({
      label: 'Pick new image',
      bytes: 0,
      undo: () => void log.push('undo'),
      redo: () => void log.push('redo'),
    }),
  );
  await act(() => undoStack.undo());
  expect(redo.disabled).toBe(false);
  await act(async () => fireEvent.click(redo));
  expect(log).toEqual(['undo', 'redo']);
  expect((screen.getByTestId('edt-toolbar-undo') as HTMLButtonElement).disabled).toBe(false);
});

test('FR-GEN-03 Export is disabled with no_image and prefetches nothing', async () => {
  const fake = await open(noImageSnapshot());
  render(
    <Tooltip.Provider>
      <ExportPanel />
    </Tooltip.Provider>,
  );
  const panel = screen.getByTestId('exp-panel');
  expect(panel.dataset.statePanel).toBe('no-image');
  if (!screen.queryByTestId('exp-panel-no-image'))
    fireEvent.click(screen.getByTestId('exp-panel-toggle'));
  expect(fake.calls.some((c) => c.cmd === 'project_prefetch_original')).toBe(false);
  expect(screen.getByTestId('exp-panel-no-image').dataset.reason).toBe('no_image');
});

test('FR-GEN-03 a project with an image still mounts the canvas', async () => {
  await open(baseSnapshot());
  render(<EditorPanel />);
  expect(screen.queryByTestId('edt-canvas-no-image')).toBeNull();
  expect(screen.getByTestId('edt-canvas-stage')).toBeTruthy();
});
