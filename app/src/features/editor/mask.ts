// Mask maths (FR-EDT-14..16, 19, 20). The mask is a w×h 8-bit buffer over the region (the inner
// rect, R1): 255 = editable (ADR 0001). It is the source of truth; the tinted canvas only mirrors
// it, so the mask the DM sees is exactly the bytes handed off (FR-EDT-18). Stamps are computed
// per pixel from the distance of the pixel centre to the dab centre, so they are deterministic
// (a 100 % hardness stroke gives exactly 255 inside and 0 outside).
import type { Rect } from './store';

export interface MaskBuffer {
  /** Region (inner rect) in image px. */
  rect: Rect;
  data: Uint8Array;
}

export const MAX_MASK_SIDE = 4096;

export type StampMode = 'paint' | 'erase';

export interface Box {
  x0: number;
  y0: number;
  x1: number; // exclusive
  y1: number; // exclusive
}

export const emptyMask = (rect: Rect): MaskBuffer => ({
  rect: { ...rect },
  data: new Uint8Array(rect.w * rect.h),
});

/** True if any mask byte is > 0 (early exit; 4 bytes per step when aligned). */
export function hasPaint(d: Uint8Array): boolean {
  const words = d.byteOffset % 4 === 0 ? d.length >>> 2 : 0;
  if (words) {
    const w = new Uint32Array(d.buffer, d.byteOffset, words);
    for (let i = 0; i < words; i++) if (w[i]) return true;
  }
  for (let i = words * 4; i < d.length; i++) if (d[i]) return true;
  return false;
}

/** Stamp alpha (0–1) at distance d for radius r and hardness h (FR-EDT-15). */
export function stampAlpha(d: number, r: number, h: number): number {
  if (r <= 0) return 0;
  const inner = h * r;
  if (d <= inner) return 1;
  if (d >= r) return 0;
  return (r - d) / (r - inner);
}

/** Dab spacing: max(1, 0.25 r) image px (FR-EDT-15). */
export const dabSpacing = (r: number): number => Math.max(1, 0.25 * r);

/** The box one dab of radius r at (cx, cy) touches (mask px, clipped), or null. */
export function dabBox(m: MaskBuffer, cx: number, cy: number, r: number): Box | null {
  const { w, h } = m.rect;
  const x0 = Math.max(0, Math.floor(cx - r));
  const y0 = Math.max(0, Math.floor(cy - r));
  const x1 = Math.min(w, Math.ceil(cx + r));
  const y1 = Math.min(h, Math.ceil(cy + r));
  return x1 <= x0 || y1 <= y0 ? null : { x0, y0, x1, y1 };
}

/** Applies one dab centred at (cx, cy) in **mask** px. Paint = source-over on alpha, erase =
 *  destination-out. Returns the touched box (clipped to the mask), or null. */
export function stamp(
  m: MaskBuffer,
  cx: number,
  cy: number,
  r: number,
  hardness: number,
  mode: StampMode,
): Box | null {
  const box = dabBox(m, cx, cy, r);
  if (!box) return null;
  const { x0, y0, x1, y1 } = box;
  const w = m.rect.w;
  const d = m.data;
  for (let y = y0; y < y1; y++) {
    const dy = y + 0.5 - cy;
    const row = y * w;
    for (let x = x0; x < x1; x++) {
      const a = stampAlpha(Math.hypot(x + 0.5 - cx, dy), r, hardness);
      if (a <= 0) continue;
      const i = row + x;
      const dst = d[i]! / 255;
      const out = mode === 'paint' ? dst + a * (1 - dst) : dst * (1 - a);
      d[i] = Math.round(out * 255);
    }
  }
  return box;
}

/** Dab centres along a segment, `carry` = distance travelled since the last dab. Returns the
 *  dabs and the new carry. The first point of a stroke is dabbed by the caller. */
export function dabsAlong(
  from: { x: number; y: number },
  to: { x: number; y: number },
  spacing: number,
  carry: number,
): { dabs: { x: number; y: number }[]; carry: number } {
  const len = Math.hypot(to.x - from.x, to.y - from.y);
  const dabs: { x: number; y: number }[] = [];
  let t = spacing - carry;
  while (t <= len) {
    const k = len === 0 ? 0 : t / len;
    dabs.push({ x: from.x + (to.x - from.x) * k, y: from.y + (to.y - from.y) * k });
    t += spacing;
  }
  return { dabs, carry: len - (t - spacing) };
}

export const unionBox = (a: Box | null, b: Box | null): Box | null =>
  !a
    ? b
    : !b
      ? a
      : {
          x0: Math.min(a.x0, b.x0),
          y0: Math.min(a.y0, b.y0),
          x1: Math.max(a.x1, b.x1),
          y1: Math.max(a.y1, b.y1),
        };

/** Copies a box of a w-wide buffer out. */
export function extract(data: Uint8Array, w: number, b: Box): Uint8Array {
  const bw = b.x1 - b.x0;
  const out = new Uint8Array(bw * (b.y1 - b.y0));
  for (let y = b.y0; y < b.y1; y++)
    out.set(data.subarray(y * w + b.x0, y * w + b.x1), (y - b.y0) * bw);
  return out;
}

/** Writes a box back (inverse of `extract`). */
export function restore(data: Uint8Array, w: number, b: Box, src: Uint8Array): void {
  const bw = b.x1 - b.x0;
  for (let y = b.y0; y < b.y1; y++)
    data.set(src.subarray((y - b.y0) * bw, (y - b.y0 + 1) * bw), y * w + b.x0);
}

/** FR-EDT-19 / FR-EDT-20: the mask on a new rect keeps the overlapping pixels; the rest is 0. */
export function remap(m: MaskBuffer, rect: Rect): MaskBuffer {
  const out = emptyMask(rect);
  const ox0 = Math.max(m.rect.x, rect.x);
  const oy0 = Math.max(m.rect.y, rect.y);
  const ox1 = Math.min(m.rect.x + m.rect.w, rect.x + rect.w);
  const oy1 = Math.min(m.rect.y + m.rect.h, rect.y + rect.h);
  for (let y = oy0; y < oy1; y++) {
    const src = (y - m.rect.y) * m.rect.w + (ox0 - m.rect.x);
    const dst = (y - rect.y) * rect.w + (ox0 - rect.x);
    out.data.set(m.data.subarray(src, src + Math.max(0, ox1 - ox0)), dst);
  }
  return out;
}

/** FNV-1a 32 of the bytes (tests compare masks without shipping them whole). */
export function maskHash(data: Uint8Array): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < data.length; i++) h = Math.imul(h ^ data[i]!, 0x01000193);
  return (h >>> 0).toString(16).padStart(8, '0');
}
