// Tile pyramid maths (ADR 0003, EDT Appendix B): level choice, tile geometry in image px, the
// wanted set for a view, and a byte-budgeted LRU. Pure: TileManager does the I/O.
import type { TilesDescriptor, TilesLevel } from '../../api';
import { screenToImage, type Size, type View } from './view';

/** FR-EDT-08: the smallest level whose scale is ≥ the device scale (scale × DPR), among the
 *  levels reported built. If none qualifies, the finest built level. `null` when nothing is built. */
export function chooseLevel(levels: readonly TilesLevel[], deviceScale: number): number | null {
  const built = levels.filter((l) => l.complete);
  if (built.length === 0) return null;
  const sharp = built.filter((l) => l.scale >= deviceScale - 1e-9);
  const pick = sharp.length
    ? sharp.reduce((a, b) => (b.scale < a.scale ? b : a))
    : built.reduce((a, b) => (b.scale > a.scale ? b : a));
  return pick.level;
}

/** The one-tile level (the backdrop, FR-EDT-07/08). */
export function backdropLevel(levels: readonly TilesLevel[]): TilesLevel | undefined {
  return levels.find((l) => l.cols === 1 && l.rows === 1) ?? levels.at(-1);
}

/** Level size in level px: ⌈size / 2ⁿ⌉. */
export function levelSize(d: Pick<TilesDescriptor, 'width' | 'height'>, level: number): Size {
  const f = 2 ** level;
  return { w: Math.ceil(d.width / f), h: Math.ceil(d.height / f) };
}

export interface TileRect {
  level: number;
  col: number;
  row: number;
  /** Image-px rectangle the tile covers. */
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A tile's rectangle in image px. A level image of ⌈W/2ⁿ⌉ px stands for the whole W, so level px
 *  map to image px by W / levelW (exact at the right and bottom edges, no padding). */
export function tileRect(d: TilesDescriptor, level: number, col: number, row: number): TileRect {
  const ls = levelSize(d, level);
  const t = d.tile_size;
  const kx = d.width / ls.w;
  const ky = d.height / ls.h;
  const px0 = col * t;
  const py0 = row * t;
  const pw = Math.min(t, ls.w - px0);
  const ph = Math.min(t, ls.h - py0);
  return { level, col, row, x: px0 * kx, y: py0 * ky, w: pw * kx, h: ph * ky };
}

/** Tiles of `level` that intersect the viewport, in row-major order from the view centre outwards. */
export function wantedTiles(
  d: TilesDescriptor,
  lv: TilesLevel,
  view: View,
  viewport: Size,
): TileRect[] {
  const a = screenToImage(view, { x: 0, y: 0 });
  const b = screenToImage(view, { x: viewport.w, y: viewport.h });
  const x0 = Math.max(0, a.x);
  const y0 = Math.max(0, a.y);
  const x1 = Math.min(d.width, b.x);
  const y1 = Math.min(d.height, b.y);
  if (x1 <= x0 || y1 <= y0) return [];
  const ls = levelSize(d, lv.level);
  const span = { x: (d.tile_size * d.width) / ls.w, y: (d.tile_size * d.height) / ls.h };
  const c0 = Math.max(0, Math.floor(x0 / span.x));
  const r0 = Math.max(0, Math.floor(y0 / span.y));
  const c1 = Math.min(lv.cols - 1, Math.ceil(x1 / span.x) - 1);
  const r1 = Math.min(lv.rows - 1, Math.ceil(y1 / span.y) - 1);
  const out: TileRect[] = [];
  for (let r = r0; r <= r1; r++)
    for (let c = c0; c <= c1; c++) out.push(tileRect(d, lv.level, c, r));
  const cx = (x0 + x1) / 2;
  const cy = (y0 + y1) / 2;
  const dist = (t: TileRect) => Math.hypot(t.x + t.w / 2 - cx, t.y + t.h / 2 - cy);
  return out.sort((p, q) => dist(p) - dist(q));
}

/** Tile URL from the descriptor's `base_url` (the UI never builds hosts itself, Appendix A). */
export function tileUrl(d: TilesDescriptor, lv: TilesLevel, col: number, row: number): string {
  return `${d.base_url}/${lv.level}/${col}_${row}.${lv.ext}`;
}

/** FR-EDT-11: decoded tiles in a byte-budgeted LRU (default 384 MiB). `pinned` keys are never
 *  evicted (the backdrop and the tiles on screen), so a full budget only drops off-screen tiles. */
export class ByteLru<V> {
  private readonly map = new Map<string, { value: V; bytes: number }>();
  private total = 0;

  constructor(
    readonly budget: number,
    private readonly dispose: (v: V, key: string) => void = () => undefined,
  ) {}

  get bytes(): number {
    return this.total;
  }

  get size(): number {
    return this.map.size;
  }

  has(key: string): boolean {
    return this.map.has(key);
  }

  keys(): string[] {
    return [...this.map.keys()];
  }

  /** Returns the value and marks it most recently used. */
  get(key: string): V | undefined {
    const e = this.map.get(key);
    if (!e) return undefined;
    this.map.delete(key);
    this.map.set(key, e);
    return e.value;
  }

  set(key: string, value: V, bytes: number, pinned: ReadonlySet<string> = new Set()): void {
    const old = this.map.get(key);
    if (old) {
      this.total -= old.bytes;
      this.map.delete(key);
      if (old.value !== value) this.dispose(old.value, key);
    }
    this.map.set(key, { value, bytes });
    this.total += bytes;
    this.evict(pinned, key);
  }

  private evict(pinned: ReadonlySet<string>, keep: string): void {
    for (const [k, e] of this.map) {
      if (this.total <= this.budget) break;
      if (k === keep || pinned.has(k)) continue;
      this.map.delete(k);
      this.total -= e.bytes;
      this.dispose(e.value, k);
    }
  }

  clear(): void {
    for (const [k, e] of this.map) this.dispose(e.value, k);
    this.map.clear();
    this.total = 0;
  }
}

export const TILE_LRU_BYTES = 384 * 1024 * 1024;
