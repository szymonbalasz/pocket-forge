// The bridge to RGN's `rgn_region_fit` (R18, FR-RGN-01/02). EDT emits `{inner}` and draws only the
// validated `{inner, outer}` that comes back. RGN owns the rules; the local fallback is gone (R39).
import { api, ApiError } from '../../api';
import type { Rect, RegionState } from './store';

export class RegionError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export async function fitRegion(
  projectId: string,
  inner: Rect,
  margin?: number,
): Promise<RegionState> {
  try {
    const f = await api.region.fit(projectId, inner, margin);
    return { inner: f.inner, outer: f.outer };
  } catch (e) {
    if (e instanceof ApiError) throw new RegionError(e.code, e.message);
    throw e;
  }
}
