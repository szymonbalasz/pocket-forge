import Konva from 'konva';
import { describe, expect, test } from 'vitest';

import type { Layer } from '../../api';
import { layerPoint, PatchLayers } from './PatchLayers';

const patch = (id: string, over: Partial<Layer> = {}): Layer => ({
  id,
  kind: 'patch',
  file: `layers/${id}.png`,
  cx: 300.25,
  cy: 180.5,
  scale: 1.37,
  rotation: 33.3,
  opacity: 0.8,
  visible: true,
  ...over,
});

function fakeLayer(): Konva.Layer {
  // A Group stands in for the Layer: same child API, no canvas (jsdom has none).
  const g = new Konva.Group();
  return Object.assign(g, { batchDraw: () => g }) as unknown as Konva.Layer;
}

// jsdom has no 2D context; Konva only needs one to exist here (nothing is rendered).
const noop2d = new Proxy(
  { getImageData: () => ({ data: new Uint8ClampedArray(4) }) } as Record<string, unknown>,
  {
    get: (t, k) => (k in t ? t[k as string] : () => undefined),
    set: (t, k, v) => ((t[k as string] = v), true),
  },
);
HTMLCanvasElement.prototype.getContext = (() => noop2d) as never;

const flush = () => new Promise((r) => setTimeout(r, 0));
const img = { w: 200, h: 120, image: {} as CanvasImageSource };

describe('patch layers', () => {
  test('FR-EDT-13 drawn corners match the layer affine within 0.01 px', async () => {
    const layer = fakeLayer();
    const p = new PatchLayers(
      layer,
      'p',
      () => undefined,
      async () => img,
    );
    const l = patch('a');
    p.setStack([l]);
    await flush();
    const node = layer.getChildren()[0]!;
    const t = node.getTransform();
    for (const [u, v] of [
      [0, 0],
      [img.w, 0],
      [img.w, img.h],
      [0, img.h],
    ] as const) {
      const got = t.point({ x: u, y: v });
      const want = layerPoint(l, img.w, img.h, u, v);
      expect(Math.abs(got.x - want.x)).toBeLessThan(0.01);
      expect(Math.abs(got.y - want.y)).toBeLessThan(0.01);
    }
    expect(node.opacity()).toBe(0.8);
  });

  test('FR-EDT-13 the stack is drawn in manifest order, hidden layers skipped, the original excluded', async () => {
    const layer = fakeLayer();
    const p = new PatchLayers(
      layer,
      'p',
      () => undefined,
      async () => img,
    );
    const original: Layer = { ...patch('o'), kind: 'original', file: null };
    p.setStack([original, patch('a'), patch('b', { visible: false }), patch('c')]);
    await flush();
    expect(p.drawn()).toEqual(['a', 'c']);
    // A commit reorders and drops a layer: the drawn stack equals the new manifest stack.
    p.setStack([original, patch('c'), patch('b')]);
    expect(p.drawn()).toEqual(['c', 'b']);
  });

  test('FR-EDT-13 a layer that fails to load is reported and the rest still draw', async () => {
    const layer = fakeLayer();
    const errors: string[] = [];
    const p = new PatchLayers(
      layer,
      'p',
      (id) => errors.push(id),
      async (_p, id) => {
        if (id === 'bad') throw new Error('HTTP 404');
        return img;
      },
    );
    p.setStack([patch('bad'), patch('ok')]);
    await flush();
    expect(errors).toEqual(['bad']);
    expect(p.drawn()).toEqual(['ok']);
  });
});
