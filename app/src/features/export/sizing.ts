// Export size maths (FR-EXP-06..08, FR-EXP-11, FR-EXP-14), a mirror of `pf-imaging`
// `sizing.rs` with the same test vectors. The core owns the real plan (`export_plan`); this copy
// keeps the mocked core honest until EXP slice 3 and lets the panel name the file it will write.
import type { CoreErrorShape, ExportFormat, ExportGrid, ExportLimit } from '../../api';

export const MIN_PX_PER_SQUARE = 8;
export const MAX_PX_PER_SQUARE = 1024;
export const MAX_CELLS = 1000;
export const MAX_SCALE = 4;
export const MAX_OUTPUT_SIDE = 16384;
export const WEBP_MAX_SIDE = 16383;
const SQUARE_TOLERANCE = 0.01;

export type SizeWarning = 'grid_not_square' | 'upscaled';

export const WARNING_TEXT: Record<SizeWarning, string> = {
  grid_not_square: "Grid isn't square",
  upscaled: 'Upscaled, softer',
};

export interface Sizing {
  source: number | null;
  target: number | null;
  /** target ÷ source, or 1 ("Full resolution"). */
  scale: number;
  width: number;
  height: number;
  warnings: SizeWarning[];
}

/** `invalid_settings{field, limit}` (EXP §9). */
export class InvalidSetting extends Error {
  constructor(
    readonly field: string,
    readonly limit: ExportLimit,
  ) {
    super(`${field} is out of range`);
  }

  toCoreError(): CoreErrorShape {
    return {
      code: 'invalid_settings',
      message: this.message,
      field: this.field,
      detail: { limit: this.limit },
    };
  }
}

function checkRange(field: string, value: number, min: number, max: number): void {
  if (Number.isNaN(value) || value < min) throw new InvalidSetting(field, { min });
  if (value > max) throw new InvalidSetting(field, { max });
}

/** Source px per square from the grid, or null when the grid gives none yet (FR-EXP-06). */
export function sourcePxPerSquare(
  grid: ExportGrid,
  width: number,
  height: number,
): { source: number | null; warning: SizeWarning | null } {
  if (grid.mode === 'px') {
    const v = grid.source_px_per_square;
    if (v === null || v === undefined) return { source: null, warning: null };
    checkRange('source_px_per_square', v, MIN_PX_PER_SQUARE, MAX_PX_PER_SQUARE);
    return { source: v, warning: null };
  }
  const { cols, rows } = grid;
  if (cols === null || cols === undefined) return { source: null, warning: null };
  checkRange('cols', cols, 1, MAX_CELLS);
  const source = width / cols;
  let warning: SizeWarning | null = null;
  if (rows !== null && rows !== undefined) {
    checkRange('rows', rows, 1, MAX_CELLS);
    if (Math.abs(source - height / rows) > SQUARE_TOLERANCE * source) warning = 'grid_not_square';
  }
  return { source, warning };
}

/** Output size and scale for a `width × height` source (FR-EXP-06..08). Throws InvalidSetting. */
export function planSize(
  width: number,
  height: number,
  grid: ExportGrid | null,
  target: number | null,
): Sizing {
  const warnings: SizeWarning[] = [];
  let source: number | null = null;
  if (grid) {
    const s = sourcePxPerSquare(grid, width, height);
    source = s.source;
    if (s.warning) warnings.push(s.warning);
  }
  let scale = 1;
  if (target !== null) {
    if (source === null)
      throw new InvalidSetting('target_px_per_square', { requires: 'source_px_per_square' });
    checkRange('target_px_per_square', target, MIN_PX_PER_SQUARE, MAX_PX_PER_SQUARE);
    scale = target / source;
  }
  if (scale > MAX_SCALE) throw new InvalidSetting('scale', { max: MAX_SCALE });
  const side = (field: string, v: number) => {
    // Rust's f64::round: halves away from zero (sides are positive, so Math.round matches).
    const out = Math.round(v * scale);
    checkRange(field, out, 1, MAX_OUTPUT_SIDE);
    return out;
  };
  const w = side('width', width);
  const h = side('height', height);
  if (scale > 1) warnings.push('upscaled');
  return { source, target, scale, width: w, height: h, warnings };
}

/** Why `format` can't encode a `width × height` export, or null when it can (FR-EXP-11). */
export function formatUnavailable(
  format: ExportFormat,
  width: number,
  height: number,
): string | null {
  return format === 'webp' && (width > WEBP_MAX_SIDE || height > WEBP_MAX_SIDE)
    ? 'WebP max 16383 px'
    : null;
}

const EXT: Record<ExportFormat, string> = { png: 'png', jpeg: 'jpg', webp: 'webp' };

/** `<project name>_<target>px.<ext>`, or `<project name>.<ext>` (FR-EXP-14). */
export function exportFileName(project: string, target: number | null, format: ExportFormat) {
  return `${project}${target === null ? '' : `_${target}px`}.${EXT[format]}`;
}

/** The project-relative destination (FR-EXP-13). */
export function exportPath(project: string, target: number | null, format: ExportFormat) {
  return `exports/${exportFileName(project, target, format)}`;
}
