// The New image tab's size block (FR-GEN-07..10): defaults per kind, input limits, the ratio
// list and the labels. The fit itself (model limit, step, nearest ratio) is the core's
// `gen_size_fit`; this module only shapes what the DM types and what the core answered. Pure.
import type { Capabilities, GenSize, GenSizeFit, ProjectKind } from '../../api';

/** FR-GEN-07 / OQ-GEN-11: Maps 30 × 40 at 70 px per square. */
export const MAP_DEFAULT = { cols: 30, rows: 40, pps: 70 } as const;
/** FR-GEN-10 / OQ-GEN-11: Tokens and Items 1:1, long side 1024. */
export const RATIO_DEFAULT = { aspect: '1:1', long_side: 1024 } as const;
export const CELLS = { min: 1, max: 200 } as const;
export const PPS = { min: 16, max: 512 } as const;

/** FR-GEN-10: the ratios a wh model offers (it lists none itself). */
export const WH_RATIOS = ['1:1', '4:5', '5:4', '3:4', '4:3', '2:3', '3:2', '9:16', '16:9'];

/** A real ratio (`2:3`), not a provider keyword such as `auto` or `source` (FR-GEN-09/10). */
export const isRatio = (v: string): boolean => /^\d+(\.\d+)?:\d+(\.\d+)?$/.test(v.trim());

export function defaultSize(kind: ProjectKind): GenSize {
  return kind === 'map' ? { mode: 'grid', ...MAP_DEFAULT } : { mode: 'ratio', ...RATIO_DEFAULT };
}

/** The saved size if it suits the kind, else the kind's default. */
export function sizeFor(kind: ProjectKind, saved: GenSize | null | undefined): GenSize {
  const want = kind === 'map' ? 'grid' : 'ratio';
  return saved && saved.mode === want ? saved : defaultSize(kind);
}

/** FR-GEN-10: aspect models offer their own ratio values (keywords excluded); wh models (and a
 *  model without `output_size`) the fixed list. */
export function ratioOptions(caps: Capabilities | null): string[] {
  const o = caps?.output_size;
  if (o?.mode === 'aspect') return o.allowed.filter(isRatio);
  return WH_RATIOS;
}

/** The long side input's bounds on a wh model (FR-GEN-10), else null (aspect: no long side). */
export function longSideLimits(
  caps: Capabilities | null,
): { min: number; max: number; step: number } | null {
  const o = caps?.output_size;
  if (o?.mode === 'aspect') return null;
  return o ? { min: o.min, max: o.max, step: o.step } : { min: 16, max: 4096, step: 16 };
}

const clampInt = (v: number, min: number, max: number) =>
  Math.min(max, Math.max(min, Math.round(Number.isFinite(v) ? v : min)));

/** The grid inputs as the core accepts them: integers, cells 1–200, pps 16–512 (FR-GEN-07). */
export function clampGrid(g: { cols: number; rows: number; pps: number }) {
  return {
    cols: clampInt(g.cols, CELLS.min, CELLS.max),
    rows: clampInt(g.rows, CELLS.min, CELLS.max),
    pps: clampInt(g.pps, PPS.min, PPS.max),
  };
}

/** A long side within the model’s limits, rounded down to its step (FR-GEN-08/10). */
export function clampLongSide(v: number, lim: { min: number; max: number; step: number }): number {
  const step = lim.step > 0 ? lim.step : 1;
  const snapped = Math.floor(clampInt(v, lim.min, lim.max) / step) * step;
  return Math.min(Math.floor(lim.max / step) * step, Math.max(lim.min, snapped));
}

const one = (n: number) => n.toFixed(1);

/** FR-GEN-08: what is sent and, for a grid, the px per square that results (1 decimal). */
export function resultLabel(size: GenSize, fit: GenSizeFit): string {
  if (fit.mode === 'aspect') return fit.aspect ? `ratio ${fit.aspect}` : '';
  if (!fit.w || !fit.h) return '';
  const sent = `${fit.w} × ${fit.h}`;
  if (size.mode !== 'grid') return sent;
  return `${sent} · ${one(fit.w / size.cols)} × ${one(fit.h / size.rows)} px per square`;
}

/** The estimate's size key (FR-GEN-20): the sent w × h, or the ratio. */
export function sizeKey(fit: GenSizeFit | null): string | null {
  if (!fit) return null;
  if (fit.mode === 'aspect') return fit.aspect ? `ratio:${fit.aspect}` : null;
  return fit.w && fit.h ? `${fit.w}x${fit.h}` : null;
}

/** FR-GEN-21: the confirm's "New image · W × H" (or the ratio). */
export function confirmSize(fit: { w?: number | null; h?: number | null; aspect?: string | null }) {
  if (fit.w && fit.h) return `${fit.w} × ${fit.h}`;
  return fit.aspect ?? '';
}
