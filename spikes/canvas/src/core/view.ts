// Single source of truth for the view transform: screen = image * scale + t.
// Image space = integer pixel grid of the original (pixel (i,j) covers [i,i+1)×[j,j+1)).
// Adapters copy this into their library; they never accumulate their own state.
export interface Pt { x: number; y: number }
export interface Rect { x: number; y: number; w: number; h: number }
export interface View { scale: number; tx: number; ty: number }

export const imageToScreen = (v: View, p: Pt): Pt => ({ x: p.x * v.scale + v.tx, y: p.y * v.scale + v.ty });
export const screenToImage = (v: View, p: Pt): Pt => ({ x: (p.x - v.tx) / v.scale, y: (p.y - v.ty) / v.scale });

/** Image pixel under a screen point (integer indices). */
export const pixelAt = (v: View, p: Pt): Pt => {
  const q = screenToImage(v, p);
  return { x: Math.floor(q.x), y: Math.floor(q.y) };
};

/** Zoom by `factor` keeping the image point under screen (sx, sy) fixed. Recomputed from the
 *  anchor each time, so repeated calls don't accumulate drift in the anchor. */
export function zoomAbout(v: View, factor: number, sx: number, sy: number, min = 1 / 128, max = 64): View {
  const scale = Math.min(max, Math.max(min, v.scale * factor));
  const ix = (sx - v.tx) / v.scale, iy = (sy - v.ty) / v.scale;
  return { scale, tx: sx - ix * scale, ty: sy - iy * scale };
}

export const panBy = (v: View, dx: number, dy: number): View => ({ ...v, tx: v.tx + dx, ty: v.ty + dy });

export function fit(r: Rect, vw: number, vh: number, margin = 0): View {
  const scale = Math.min((vw - 2 * margin) / r.w, (vh - 2 * margin) / r.h);
  return { scale, tx: (vw - r.w * scale) / 2 - r.x * scale, ty: (vh - r.h * scale) / 2 - r.y * scale };
}

/** View that shows image point (ix, iy) at screen (sx, sy) with the given scale. */
export const centerOn = (scale: number, ix: number, iy: number, sx: number, sy: number): View =>
  ({ scale, tx: sx - ix * scale, ty: sy - iy * scale });

/** Visible image rect for a viewport. */
export function visibleRect(v: View, vw: number, vh: number): Rect {
  const a = screenToImage(v, { x: 0, y: 0 }), b = screenToImage(v, { x: vw, y: vh });
  return { x: a.x, y: a.y, w: b.x - a.x, h: b.y - a.y };
}
