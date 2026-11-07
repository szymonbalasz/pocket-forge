// Region tool geometry (FR-EDT-27, R18): draw a new region, move it, resize it by 8 handles.
// Works in image px; the result is integer image px clamped to the image, emitted as `{inner}`.
import type { Rect } from './store';
import { imageToScreen, type Pt, type Size, type View } from './view';

export type Handle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';
export const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

export type RegionDrag =
  | { kind: 'new'; start: Pt }
  | { kind: 'move'; start: Pt; rect: Rect }
  | { kind: 'resize'; handle: Handle; rect: Rect };

/** Integer rect from two image points, clamped to the image (min 1×1). */
export function rectFromPoints(a: Pt, b: Pt, image: Size): Rect {
  const cl = (v: number, max: number) => Math.min(max, Math.max(0, v));
  const x0 = Math.floor(cl(Math.min(a.x, b.x), image.w));
  const y0 = Math.floor(cl(Math.min(a.y, b.y), image.h));
  const x1 = Math.ceil(cl(Math.max(a.x, b.x), image.w));
  const y1 = Math.ceil(cl(Math.max(a.y, b.y), image.h));
  return { x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0) };
}

/** Moves a rect by (dx, dy) image px, kept inside the image, integer. */
export function moveRect(r: Rect, dx: number, dy: number, image: Size): Rect {
  const x = Math.round(Math.min(image.w - r.w, Math.max(0, r.x + dx)));
  const y = Math.round(Math.min(image.h - r.h, Math.max(0, r.y + dy)));
  return { ...r, x, y };
}

/** Resizes by dragging `handle` to image point p; the opposite edges stay put. */
export function resizeRect(r: Rect, handle: Handle, p: Pt, image: Size): Rect {
  let x0 = r.x;
  let y0 = r.y;
  let x1 = r.x + r.w;
  let y1 = r.y + r.h;
  if (handle.includes('w')) x0 = p.x;
  if (handle.includes('e')) x1 = p.x;
  if (handle.includes('n')) y0 = p.y;
  if (handle.includes('s')) y1 = p.y;
  return rectFromPoints({ x: x0, y: y0 }, { x: x1, y: y1 }, image);
}

export function applyDrag(d: RegionDrag, p: Pt, image: Size): Rect {
  switch (d.kind) {
    case 'new':
      return rectFromPoints(d.start, p, image);
    case 'move':
      return moveRect(d.rect, p.x - d.start.x, p.y - d.start.y, image);
    case 'resize':
      return resizeRect(d.rect, d.handle, p, image);
  }
}

/** Handle centres in screen px (CSS). */
export function handlePoints(r: Rect, v: View): Record<Handle, Pt> {
  const a = imageToScreen(v, { x: r.x, y: r.y });
  const b = imageToScreen(v, { x: r.x + r.w, y: r.y + r.h });
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  return {
    nw: a,
    n: { x: mx, y: a.y },
    ne: { x: b.x, y: a.y },
    e: { x: b.x, y: my },
    se: b,
    s: { x: mx, y: b.y },
    sw: { x: a.x, y: b.y },
    w: { x: a.x, y: my },
  };
}

export const insideRect = (r: Rect, p: Pt): boolean =>
  p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h;

export const sameRect = (a: Rect | null | undefined, b: Rect | null | undefined): boolean =>
  !!a && !!b && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
