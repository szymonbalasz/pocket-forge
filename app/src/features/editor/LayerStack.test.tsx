// BL-22 (OQ-PRJ-11 A): the read-only layer-stack drawer against the test core: the core's files
// placed with the manifest affine, hidden layers skipped, the caption alone when a file is missing.
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'vitest';

import type { Layer } from '../../api';
import { installTestCore } from '../../api/mocks/testCore';
import { LayerStack, STACK_ORIGINAL_MAX_SIDE } from './LayerStack';

const core = installTestCore();
const PID = '01JPROJECT0000000000000000';

const layer = (id: string, kind: Layer['kind'], patch: Partial<Layer> = {}): Layer => ({
  id,
  kind,
  file: kind === 'patch' ? `.pforge/layers/${id}.png` : null,
  cx: 1000,
  cy: 500,
  scale: 1,
  rotation: 0,
  opacity: 1,
  visible: true,
  ...patch,
});

// A 2000×1000 original; a 400×200 patch at (500, 250), ×0.5, 30°, 80 %; a hidden patch.
const stack = [
  layer('ORIG', 'original'),
  layer('PA', 'patch', { cx: 500, cy: 250, scale: 0.5, rotation: 30, opacity: 0.8 }),
  layer('PB', 'patch', { visible: false }),
];

function useCore(fail: string[] = []) {
  core.useHandlers({
    edt_image_open: (input) => {
      const id = String(input.image_id);
      if (fail.includes(id)) throw { code: 'io', message: 'not here' };
      const orig = id === 'ORIG';
      return {
        key: `k-${id}`,
        url: `http://pf-tiles.localhost/i/k-${id}.png`,
        width: orig ? 1024 : 400,
        height: orig ? 512 : 200,
      };
    },
  });
}

const caption = <p data-testid="cap">Original, Patch 1</p>;
const opened = () => core.calls.filter((c) => c.cmd === 'edt_image_open').map((c) => c.input);

afterEach(cleanup);

test('BL-22 OQ-PRJ-11 A draws the original and each visible patch with the manifest affine', async () => {
  useCore();
  render(<LayerStack projectId={PID} layers={stack} testId="s" caption={caption} />);
  await waitFor(() => expect(screen.getByTestId('s').dataset.state).toBe('drawn'));
  const original = screen.getByTestId('s-original');
  expect(original.getAttribute('src')).toBe('http://pf-tiles.localhost/i/k-ORIG.png');
  const patches = screen.getAllByTestId('s-layer');
  expect(patches.map((p) => p.dataset.recordId)).toEqual(['PA']);
  const st = patches[0]!.style;
  expect([st.left, st.top, st.width, st.height]).toEqual(['25%', '25%', '10%', '10%']);
  expect(st.opacity).toBe('0.8');
  expect(st.transform).toBe('translate(-50%, -50%) rotate(30deg)');
  expect(screen.getByTestId('cap')).toBeTruthy();
  // The original from a cached copy; patches as baked, whole; the hidden one isn't requested.
  expect(opened()).toEqual([
    { project_id: PID, image_id: 'ORIG', max_side: STACK_ORIGINAL_MAX_SIDE },
    { project_id: PID, image_id: 'PA' },
  ]);
});

test('BL-22 a layer file known missing shows only the caption and is not requested', async () => {
  useCore();
  render(
    <LayerStack
      projectId={PID}
      layers={stack}
      missingFiles={['.pforge/layers/PA.png']}
      testId="s"
      caption={caption}
    />,
  );
  expect(screen.getByTestId('s').dataset.state).toBe('fallback');
  expect(screen.getByTestId('cap')).toBeTruthy();
  expect(screen.queryByTestId('s-original')).toBeNull();
  await waitFor(() => expect(opened().map((i) => i.image_id)).toEqual(['ORIG']));
});

test('BL-22 a layer the core cannot open falls back to the caption', async () => {
  useCore(['PA']);
  render(<LayerStack projectId={PID} layers={stack} testId="s" caption={caption} />);
  await waitFor(() => expect(screen.getByTestId('s').dataset.state).toBe('fallback'));
  expect(screen.queryByTestId('s-layer')).toBeNull();
  expect(screen.getByTestId('cap')).toBeTruthy();
});

test('BL-22 an image that fails to decode falls back to the caption', async () => {
  useCore();
  render(<LayerStack projectId={PID} layers={stack} testId="s" caption={caption} />);
  await waitFor(() => expect(screen.getByTestId('s').dataset.state).toBe('drawn'));
  fireEvent.error(screen.getByTestId('s-layer'));
  expect(screen.getByTestId('s').dataset.state).toBe('fallback');
});

test('BL-22 an unreadable version (no layers) shows only the caption', () => {
  useCore();
  render(<LayerStack projectId={PID} layers={[]} testId="s" caption={caption} />);
  expect(screen.getByTestId('s').dataset.state).toBe('fallback');
  expect(opened()).toEqual([]);
});
