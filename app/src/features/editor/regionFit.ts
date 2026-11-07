// The bridge to RGN's `rgn_region_fit` (R18, FR-RGN-01/02). EDT emits `{inner}` and draws only the
// validated `{inner, outer}` that comes back. Until region-edit lands, a missing command falls
// back to a local mock of FR-RGN-01/02 (snap, clamp, ≥ 16 px, 128 px margin, outer ≤ 4096).
import { api, ApiError } from '../../api';
import type { Rect, RegionState } from './store';
import type { Size } from './view';

export const DEFAULT_MARGIN = 128;

export class RegionError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** Local stand-in for `rgn_region_fit` (mock; RGN owns the real rules). */
export function mockRegionFit(inner: Rect, image: Size, margin = DEFAULT_MARGIN): RegionState {
  const x0 = Math.max(0, Math.round(inner.x));
  const y0 = Math.max(0, Math.round(inner.y));
  const x1 = Math.min(image.w, Math.round(inner.x + inner.w));
  const y1 = Math.min(image.h, Math.round(inner.y + inner.h));
  if (x1 <= x0 || y1 <= y0)
    throw new RegionError('out_of_bounds', 'The region is outside the image.');
  const r = { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  if (r.w < 16 || r.h < 16)
    throw new RegionError('region_too_small', 'A region is at least 16 × 16 px.');
  const ox0 = Math.max(0, r.x - margin);
  const oy0 = Math.max(0, r.y - margin);
  const ox1 = Math.min(image.w, r.x + r.w + margin);
  const oy1 = Math.min(image.h, r.y + r.h + margin);
  const outer = { x: ox0, y: oy0, w: ox1 - ox0, h: oy1 - oy0 };
  if (outer.w > 4096 || outer.h > 4096)
    throw new RegionError(
      'region_too_large',
      'The region plus its context margin is over 4096 px.',
    );
  return { inner: r, outer };
}

const missing = (e: unknown) =>
  e instanceof ApiError &&
  (e.code === 'ipc' || (e.code === 'not_found' && /unknown command/.test(e.message)));

export async function fitRegion(
  projectId: string,
  inner: Rect,
  image: Size,
  margin?: number,
): Promise<RegionState> {
  try {
    const f = await api.region.fit(projectId, inner, margin);
    return { inner: f.inner, outer: f.outer };
  } catch (e) {
    if (missing(e)) return mockRegionFit(inner, image, margin);
    if (e instanceof ApiError) throw new RegionError(e.code, e.message);
    throw e;
  }
}
