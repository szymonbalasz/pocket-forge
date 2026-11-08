// The mask's on-screen form (FR-EDT-14): CPU-backed canvases (`willReadFrequently`) whose alpha is
// the mask value, drawn by Konva.Images over the region at 50 % opacity.
//
// Speed at the largest mask (EDT §12, BL-09): the canvas is split into 512² tiles, so a dab or an
// undo step re-uploads only the tiles it touched instead of one 3840² texture; changes are
// coalesced into one repaint per frame; and pixels are written as packed 32-bit words through a
// 256-entry table. The mask layer has smoothing off, so tile edges don't show.
import Konva from 'konva';

import type { Box, MaskBuffer } from './mask';
import type { MaskView } from './MaskController';

/** Side of one tint tile, in mask px. */
export const MASK_TILE = 512;

function tintRgb(): [number, number, number] {
  const css = getComputedStyle(document.documentElement).getPropertyValue('--pf-accent').trim();
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(css);
  return m ? [parseInt(m[1]!, 16), parseInt(m[2]!, 16), parseInt(m[3]!, 16)] : [74, 144, 226];
}

/** RGBA words for each mask value (ImageData is RGBA bytes; the word layout is little-endian). */
function tintTable([r, g, b]: [number, number, number]): Uint32Array {
  const lut = new Uint32Array(256);
  const le = new Uint8Array(new Uint32Array([1]).buffer)[0] === 1;
  for (let a = 0; a < 256; a++)
    lut[a] = le
      ? ((a << 24) | (b << 16) | (g << 8) | r) >>> 0
      : ((r << 24) | (g << 16) | (b << 8) | a) >>> 0;
  return lut;
}

/** The tiles a mask of w×h is split into. */
export function maskTiles(w: number, h: number, side = MASK_TILE): Box[] {
  const out: Box[] = [];
  for (let y0 = 0; y0 < h; y0 += side)
    for (let x0 = 0; x0 < w; x0 += side)
      out.push({ x0, y0, x1: Math.min(w, x0 + side), y1: Math.min(h, y0 + side) });
  return out;
}

interface Tile {
  box: Box;
  ctx: CanvasRenderingContext2D | null;
}

export function konvaMaskView(
  layer: Konva.Layer,
): MaskView & { node: () => Konva.Group | null; destroy: () => void } {
  let group: Konva.Group | null = null;
  let tiles: Tile[] = [];
  let mask: MaskBuffer | null = null;
  let dirty: Box | null = null;
  let frame: number | null = null;
  const lut = tintTable(tintRgb());

  const paint = (m: MaskBuffer, b: Box) => {
    for (const t of tiles) {
      const x0 = Math.max(b.x0, t.box.x0);
      const y0 = Math.max(b.y0, t.box.y0);
      const x1 = Math.min(b.x1, t.box.x1);
      const y1 = Math.min(b.y1, t.box.y1);
      const w = x1 - x0;
      const h = y1 - y0;
      if (w <= 0 || h <= 0 || !t.ctx) continue;
      const img = t.ctx.createImageData(w, h);
      const px = new Uint32Array(img.data.buffer);
      const data = m.data;
      for (let y = 0; y < h; y++) {
        const src = (y0 + y) * m.rect.w + x0;
        const dst = y * w;
        for (let x = 0; x < w; x++) px[dst + x] = lut[data[src + x]!]!;
      }
      t.ctx.putImageData(img, x0 - t.box.x0, y0 - t.box.y0);
    }
  };

  const flush = () => {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
    if (mask && dirty) paint(mask, dirty);
    dirty = null;
  };

  const drop = () => {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
    dirty = null;
    group?.destroy();
    group = null;
    tiles = [];
    mask = null;
  };

  return {
    node: () => group,
    reset(m) {
      drop();
      if (m) {
        mask = m;
        group = new Konva.Group({
          x: m.rect.x,
          y: m.rect.y,
          opacity: 0.5,
          name: 'mask-tint',
          listening: false,
        });
        tiles = maskTiles(m.rect.w, m.rect.h).map((box) => {
          const canvas = document.createElement('canvas');
          canvas.width = box.x1 - box.x0;
          canvas.height = box.y1 - box.y0;
          group!.add(
            new Konva.Image({
              image: canvas,
              x: box.x0,
              y: box.y0,
              width: canvas.width,
              height: canvas.height,
              // Each tile is an Image named "mask" (the e2e contract for the tint).
              name: 'mask mask-tile',
              listening: false,
              perfectDrawEnabled: false,
            }),
          );
          return { box, ctx: canvas.getContext('2d', { willReadFrequently: true }) };
        });
        paint(m, { x0: 0, y0: 0, x1: m.rect.w, y1: m.rect.h });
        layer.add(group);
      }
      layer.batchDraw();
    },
    update(m, b) {
      if (m !== mask) return;
      dirty = dirty
        ? {
            x0: Math.min(dirty.x0, b.x0),
            y0: Math.min(dirty.y0, b.y0),
            x1: Math.max(dirty.x1, b.x1),
            y1: Math.max(dirty.y1, b.y1),
          }
        : { ...b };
      // One repaint and one layer draw per frame, however many dabs land in it.
      frame ??= requestAnimationFrame(() => {
        frame = null;
        flush();
        layer.draw();
      });
    },
    /** Drops the tint nodes and canvases (the layer is being replaced or unmounted). */
    destroy: drop,
  };
}
