// Library-agnostic tile pyramid manager. Decides which tiles to show for a view, loads them
// (fetch → createImageBitmap, decoded off the main thread) and hands them to a sink (the adapter).
import { View, visibleRect } from './view';

export interface Level { level: number; scale: number; width: number; height: number; cols: number; rows: number }
export interface Manifest { width: number; height: number; tileSize: number; ext: string; maxLevel: number; levels: Level[] }

export interface TileSink {
  addTile(key: string, bmp: ImageBitmap, x: number, y: number, w: number, h: number, z: 0 | 1): void;
  removeTile(key: string): void;
}

export type TileFetcher = (level: number, col: number, row: number) => Promise<ImageBitmap>;

export const httpFetcher = (base: string, ext: string): TileFetcher => async (l, c, r) => {
  const res = await fetch(`${base}/${l}/${c}_${r}.${ext}`);
  if (!res.ok) throw new Error(`tile ${l}/${c}_${r}: ${res.status}`);
  return createImageBitmap(await res.blob());
};

interface Entry { bmp: ImageBitmap; bytes: number; used: number }

export class TileManager {
  readonly baseLevel: number;
  private cache = new Map<string, Entry>();
  private loading = new Set<string>();
  private queue: { key: string; l: number; c: number; r: number }[] = [];
  private ready: string[] = [];
  private shown = new Set<string>();
  private wanted = new Set<string>();
  private active = 0;
  private frame = 0;
  bytes = 0;
  loads = 0;
  errors = 0;

  constructor(
    readonly m: Manifest,
    private fetcher: TileFetcher,
    private sink: TileSink,
    private opts = { concurrency: 8, cacheBytes: 384 * 2 ** 20, maxAddsPerFrame: 16 },
  ) {
    // Base level: largest level that fits in one tile per side; always kept as a backdrop.
    this.baseLevel = m.levels.filter(l => l.cols === 1 && l.rows === 1).at(-1)!.level;
  }

  levelFor(scale: number): Level {
    const lv = this.m.levels.find(l => l.scale >= scale * 0.999) ?? this.m.levels[this.m.maxLevel];
    return lv;
  }

  private rect(lv: Level, c: number, r: number) {
    const ts = this.m.tileSize, inv = 1 / lv.scale;
    const w = Math.min(ts, lv.width - c * ts), h = Math.min(ts, lv.height - r * ts);
    return { x: c * ts * inv, y: r * ts * inv, w: w * inv, h: h * inv };
  }

  /** Call once per frame before rendering. */
  update(v: View, vw: number, vh: number) {
    this.frame++;
    const want = new Map<string, { l: number; c: number; r: number; z: 0 | 1 }>();
    want.set(`${this.baseLevel}/0_0`, { l: this.baseLevel, c: 0, r: 0, z: 0 });
    const lv = this.levelFor(v.scale);
    if (lv.level !== this.baseLevel) {
      const vr = visibleRect(v, vw, vh), ts = this.m.tileSize / lv.scale;
      const c0 = Math.max(0, Math.floor(vr.x / ts)), c1 = Math.min(lv.cols - 1, Math.floor((vr.x + vr.w) / ts));
      const r0 = Math.max(0, Math.floor(vr.y / ts)), r1 = Math.min(lv.rows - 1, Math.floor((vr.y + vr.h) / ts));
      for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) want.set(`${lv.level}/${c}_${r}`, { l: lv.level, c, r, z: 1 });
    }
    this.wanted = new Set(want.keys());
    // Hide tiles no longer wanted.
    for (const k of this.shown) if (!this.wanted.has(k)) { this.sink.removeTile(k); this.shown.delete(k); }
    // Show cached ones, queue missing ones (drop stale queued requests).
    this.queue = this.queue.filter(q => this.wanted.has(q.key));
    for (const [k, t] of want) {
      const e = this.cache.get(k);
      if (e) {
        e.used = this.frame;
        if (!this.shown.has(k)) this.show(k, e.bmp, t.l, t.c, t.r, t.z);
      } else if (!this.loading.has(k) && !this.queue.some(q => q.key === k)) this.queue.push({ key: k, ...t });
    }
    // Newly decoded tiles: bounded adds per frame (texture uploads happen at render).
    let adds = 0;
    while (this.ready.length && adds < this.opts.maxAddsPerFrame) {
      const k = this.ready.shift()!;
      const e = this.cache.get(k);
      if (!e || !this.wanted.has(k) || this.shown.has(k)) continue;
      const [l, cr] = k.split('/'); const [c, r] = cr.split('_').map(Number);
      this.show(k, e.bmp, +l, c, r, +l === this.baseLevel ? 0 : 1);
      adds++;
    }
    this.pump();
    this.evict();
  }

  private show(k: string, bmp: ImageBitmap, l: number, c: number, r: number, z: 0 | 1) {
    const rc = this.rect(this.m.levels[l], c, r);
    this.sink.addTile(k, bmp, rc.x, rc.y, rc.w, rc.h, z);
    this.shown.add(k);
  }

  private pump() {
    while (this.active < this.opts.concurrency && this.queue.length) {
      const q = this.queue.shift()!;
      this.active++; this.loading.add(q.key);
      this.fetcher(q.l, q.c, q.r).then(bmp => {
        const bytes = bmp.width * bmp.height * 4;
        this.cache.set(q.key, { bmp, bytes, used: this.frame });
        this.bytes += bytes; this.loads++;
        this.ready.push(q.key);
      }, () => { this.errors++; }).finally(() => { this.active--; this.loading.delete(q.key); });
    }
  }

  private evict() {
    if (this.bytes <= this.opts.cacheBytes) return;
    const lru = [...this.cache].filter(([k]) => !this.shown.has(k)).sort((a, b) => a[1].used - b[1].used);
    for (const [k, e] of lru) {
      if (this.bytes <= this.opts.cacheBytes * 0.8) break;
      this.cache.delete(k); this.bytes -= e.bytes; e.bmp.close();
    }
  }

  /** True when every wanted tile is shown. */
  get settled() { return [...this.wanted].every(k => this.shown.has(k)); }
  get shownCount() { return this.shown.size; }

  destroy() {
    for (const k of this.shown) this.sink.removeTile(k);
    for (const e of this.cache.values()) e.bmp.close();
    this.cache.clear(); this.shown.clear();
  }
}
