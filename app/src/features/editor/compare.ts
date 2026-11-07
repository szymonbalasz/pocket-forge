// Compare geometry (FR-EDT-24): the after image covers `rect` in image px at any zoom; in split
// mode it shows to the right of the divider, which sits at `split` (0–1) across the rect.
import type { Rect } from './store';
import { imageToScreen, screenToImage, type View } from './view';

/** Image-px x of the divider. */
export const dividerImageX = (r: Rect, split: number): number => r.x + r.w * split;

/** The visible part of the after image (image px). Flip: all of it, or nothing while held. */
export function afterClip(r: Rect, mode: 'split' | 'flip', split: number, before: boolean): Rect {
  if (mode === 'flip') return before ? { ...r, w: 0 } : r;
  const x = dividerImageX(r, split);
  return { x, y: r.y, w: r.x + r.w - x, h: r.h };
}

/** Divider in screen px (CSS): x and the vertical span. */
export function dividerScreen(v: View, r: Rect, split: number) {
  const top = imageToScreen(v, { x: dividerImageX(r, split), y: r.y });
  return { x: top.x, y: top.y, h: r.h * v.scale };
}

/** Split from a screen x (CSS) while dragging the divider, clamped to 0–1. */
export function splitFromScreen(v: View, r: Rect, screenX: number): number {
  const p = screenToImage(v, { x: screenX, y: 0 });
  return Math.min(1, Math.max(0, (p.x - r.x) / r.w));
}
