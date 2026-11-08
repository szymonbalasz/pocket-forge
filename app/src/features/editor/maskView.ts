// The mask's on-screen form (FR-EDT-14): a CPU-backed canvas (`willReadFrequently`) whose alpha is
// the mask value, drawn by a Konva.Image over the region at 50 % opacity.
import Konva from 'konva';

import type { Box, MaskBuffer } from './mask';
import type { MaskView } from './MaskController';

function tintRgb(): [number, number, number] {
  const css = getComputedStyle(document.documentElement).getPropertyValue('--pf-accent').trim();
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(css);
  return m ? [parseInt(m[1]!, 16), parseInt(m[2]!, 16), parseInt(m[3]!, 16)] : [74, 144, 226];
}

export function konvaMaskView(
  layer: Konva.Layer,
): MaskView & { node: () => Konva.Image | null; destroy: () => void } {
  let canvas: HTMLCanvasElement | null = null;
  let ctx: CanvasRenderingContext2D | null = null;
  let node: Konva.Image | null = null;
  const rgb = tintRgb();

  const paint = (m: MaskBuffer, b: Box) => {
    if (!ctx) return;
    const w = b.x1 - b.x0;
    const h = b.y1 - b.y0;
    if (w <= 0 || h <= 0) return;
    const img = ctx.createImageData(w, h);
    const px = img.data;
    for (let y = 0; y < h; y++) {
      const src = (b.y0 + y) * m.rect.w + b.x0;
      for (let x = 0; x < w; x++) {
        const o = (y * w + x) * 4;
        px[o] = rgb[0];
        px[o + 1] = rgb[1];
        px[o + 2] = rgb[2];
        px[o + 3] = m.data[src + x]!;
      }
    }
    ctx.putImageData(img, b.x0, b.y0);
  };

  return {
    node: () => node,
    reset(m) {
      node?.destroy();
      node = null;
      canvas = null;
      ctx = null;
      if (m) {
        canvas = document.createElement('canvas');
        canvas.width = m.rect.w;
        canvas.height = m.rect.h;
        ctx = canvas.getContext('2d', { willReadFrequently: true });
        paint(m, { x0: 0, y0: 0, x1: m.rect.w, y1: m.rect.h });
        node = new Konva.Image({
          image: canvas,
          x: m.rect.x,
          y: m.rect.y,
          width: m.rect.w,
          height: m.rect.h,
          opacity: 0.5,
          name: 'mask',
          listening: false,
        });
        layer.add(node);
      }
      layer.batchDraw();
    },
    update(m, b) {
      paint(m, b);
      layer.batchDraw();
    },
    /** Drops the tint node and canvas (the layer is being replaced or unmounted). */
    destroy() {
      node?.destroy();
      node = null;
      canvas = null;
      ctx = null;
    },
  };
}
