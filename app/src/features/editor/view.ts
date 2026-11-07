// The one view transform (ADR 0003, FR-EDT-06): screen = image × scale + (tx, ty), in CSS px.
// Image space is the original's integer pixel grid. Pure functions; Konva only copies the result.
export interface Pt {
  x: number;
  y: number;
}
export interface View {
  scale: number;
  tx: number;
  ty: number;
}
export interface Size {
  w: number;
  h: number;
}

export const imageToScreen = (v: View, p: Pt): Pt => ({
  x: p.x * v.scale + v.tx,
  y: p.y * v.scale + v.ty,
});

export const screenToImage = (v: View, p: Pt): Pt => ({
  x: (p.x - v.tx) / v.scale,
  y: (p.y - v.ty) / v.scale,
});

/** FR-EDT-01: the whole image, centred, inside the viewport. */
export function fit(image: Size, viewport: Size): View {
  const scale = Math.min(viewport.w / image.w, viewport.h / image.h);
  return {
    scale,
    tx: (viewport.w - image.w * scale) / 2,
    ty: (viewport.h - image.h * scale) / 2,
  };
}

/** FR-EDT-02: one wheel notch (deltaY 100) scales by 2^¼. */
export const wheelFactor = (deltaY: number): number => 2 ** (-deltaY / 400);

/** FR-EDT-02 clamp: [0.5 × fit, 32 device px per image px]. */
export function scaleLimits(
  image: Size,
  viewport: Size,
  dpr: number,
): { min: number; max: number } {
  return { min: 0.5 * fit(image, viewport).scale, max: 32 / dpr };
}

/** Zoom keeping the image point under screen point `anchor` fixed (recomputed from the anchor,
 *  so repeated steps don't drift). */
export function zoomAbout(v: View, factor: number, anchor: Pt, min: number, max: number): View {
  const scale = Math.min(max, Math.max(min, v.scale * factor));
  const p = screenToImage(v, anchor);
  return { scale, tx: anchor.x - p.x * scale, ty: anchor.y - p.y * scale };
}

/** FR-EDT-03: pan, keeping at least `keep` CSS px of the image inside the viewport. */
export function panBy(
  v: View,
  dx: number,
  dy: number,
  image: Size,
  viewport: Size,
  keep = 64,
): View {
  const w = image.w * v.scale;
  const h = image.h * v.scale;
  const clamp = (t: number, size: number, port: number) =>
    Math.min(port - Math.min(keep, size), Math.max(Math.min(keep, size) - size, t));
  return {
    scale: v.scale,
    tx: clamp(v.tx + dx, w, viewport.w),
    ty: clamp(v.ty + dy, h, viewport.h),
  };
}

/** Readout: image px : device px as a percentage (FR-EDT-04). */
export const zoomPercent = (v: View, dpr: number): number => Math.round(v.scale * dpr * 100);

/** Zoom to an absolute scale, keeping the image point under `anchor` fixed (presets). */
export function zoomTo(v: View, scale: number, anchor: Pt): View {
  const p = screenToImage(v, anchor);
  return { scale, tx: anchor.x - p.x * scale, ty: anchor.y - p.y * scale };
}

/** FR-EDT-04 Actual pixels: 1 image px = 1 device px. */
export const actualScale = (dpr: number): number => 1 / dpr;

/** FR-EDT-04 Display size (R23): 1 image px = target ÷ source px per square, in **CSS** px.
 *  `null` until both values exist (R6). */
export function displayScale(source: number | null, target: number | null): number | null {
  if (source === null || target === null || !(source > 0) || !(target > 0)) return null;
  return target / source;
}
