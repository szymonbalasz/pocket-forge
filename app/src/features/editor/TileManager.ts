// The tile manager (ADR 0003, FR-EDT-07/08/11/12): fetches tiles from the pf-tiles scheme,
// decodes them off the main thread (createImageBitmap), keeps them in a 384 MiB LRU and draws
// them imperatively into a Konva layer (OQ-CNV-06): the one-tile backdrop always, then any cached
// intermediate levels coarse to fine, then the current level.
import Konva from 'konva';

import type { TilesDescriptor, TilesLevel } from '../../api';
import {
  backdropLevel,
  ByteLru,
  chooseLevel,
  TILE_LRU_BYTES,
  tileRect,
  tileUrl,
  wantedTiles,
  type TileRect,
} from './tiles';
import type { Size, View } from './view';

/** FR-EDT-12: retry delays after the first failure. */
export const RETRY_DELAYS_MS = [250, 1000];

export type FetchTile = (url: string) => Promise<ImageBitmap>;

export const fetchTile: FetchTile = async (url) => {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return createImageBitmap(await res.blob());
};

/** Where tiles are drawn. `konvaSink` in the app; a fake in unit tests (jsdom has no canvas). */
export interface TileSink {
  show(url: string, level: number, rect: TileRect, bitmap: ImageBitmap): void;
  hide(url: string): void;
  has(url: string): boolean;
  urls(): string[];
  draw(): void;
  destroy(): void;
}

/** Draws tiles as Konva.Image nodes, one group per level, coarse levels below fine ones. */
export function konvaSink(layer: Konva.Layer): TileSink {
  const nodes = new Map<string, Konva.Image>();
  const groups = new Map<number, Konva.Group>();
  const group = (level: number) => {
    let g = groups.get(level);
    if (!g) {
      g = new Konva.Group({ name: `tiles-level-${level}`, listening: false });
      layer.add(g);
      groups.set(level, g);
      [...groups.entries()].sort((a, b) => b[0] - a[0]).forEach(([, grp], i) => grp.zIndex(i));
    }
    return g;
  };
  return {
    show(url, level, t, bitmap) {
      const old = nodes.get(url);
      if (old?.image() === bitmap) return;
      old?.destroy();
      const node = new Konva.Image({
        image: bitmap,
        x: t.x,
        y: t.y,
        width: t.w,
        height: t.h,
        name: `tile tile-level-${level}`,
        id: url,
        listening: false,
        perfectDrawEnabled: false,
      });
      group(level).add(node);
      nodes.set(url, node);
    },
    hide(url) {
      nodes.get(url)?.destroy();
      nodes.delete(url);
    },
    has: (url) => nodes.has(url),
    urls: () => [...nodes.keys()],
    draw: () => layer.batchDraw(),
    destroy() {
      for (const g of groups.values()) g.destroy();
      nodes.clear();
      groups.clear();
    },
  };
}

export interface TileStats {
  /** Level drawn on top (FR-EDT-08), or null before anything is built. */
  level: number | null;
  backdropLevel: number | null;
  backdropShown: boolean;
  /** URLs of the tiles currently drawn. */
  shown: string[];
  cachedBytes: number;
  failed: number;
}

export class TileManager {
  private desc: TilesDescriptor;
  private readonly lru: ByteLru<ImageBitmap>;
  private readonly inflight = new Set<string>();
  private readonly failed = new Set<string>();
  private last: { view: View; viewport: Size; dpr: number } | null = null;
  private level: number | null = null;
  private destroyed = false;
  private generation = 0;
  private timers = new Set<ReturnType<typeof setTimeout>>();

  constructor(
    private readonly sink: TileSink,
    desc: TilesDescriptor,
    private readonly onChange: (s: TileStats) => void,
    private readonly fetcher: FetchTile = fetchTile,
    budget = TILE_LRU_BYTES,
  ) {
    this.desc = desc;
    this.lru = new ByteLru<ImageBitmap>(budget, (b, url) => this.dispose(url, b));
  }

  get descriptor(): TilesDescriptor {
    return this.desc;
  }

  /** A level finished building (`edt:tiles-progress`). */
  markBuilt(level: number): void {
    const lv = this.desc.levels.find((l) => l.level === level);
    if (!lv || lv.complete) return;
    this.desc = {
      ...this.desc,
      levels: this.desc.levels.map((l) => (l.level === level ? { ...l, complete: true } : l)),
    };
    this.refresh();
  }

  /** After a rebuild: new descriptor, failed tiles may be fetched again. */
  reset(desc: TilesDescriptor): void {
    // Drop pending retries and in-flight results of the old pyramid.
    this.generation++;
    for (const t of this.timers) clearTimeout(t);
    this.timers.clear();
    this.inflight.clear();
    this.failed.clear();
    this.desc = desc;
    for (const u of this.sink.urls()) this.sink.hide(u);
    this.lru.clear();
    this.refresh();
  }

  update(view: View, viewport: Size, dpr: number): void {
    this.last = { view, viewport, dpr };
    this.refresh();
  }

  stats(): TileStats {
    const bd = backdropLevel(this.desc.levels);
    return {
      level: this.level,
      backdropLevel: bd?.level ?? null,
      backdropShown: !!bd && this.sink.has(tileUrl(this.desc, bd, 0, 0)),
      shown: this.sink.urls(),
      cachedBytes: this.lru.bytes,
      failed: this.failed.size,
    };
  }

  destroy(): void {
    this.destroyed = true;
    for (const t of this.timers) clearTimeout(t);
    this.lru.clear();
    this.sink.destroy();
  }

  private refresh(): void {
    if (this.destroyed || !this.last) return;
    const { view, viewport, dpr } = this.last;
    const d = this.desc;
    const bd = backdropLevel(d.levels);
    this.level = chooseLevel(d.levels, view.scale * dpr);

    // What to draw, coarse to fine; only the backdrop and the current level are fetched.
    const draw: { lv: TilesLevel; t: TileRect; fetch: boolean }[] = [];
    if (bd?.complete) draw.push({ lv: bd, t: tileRect(d, bd.level, 0, 0), fetch: true });
    if (this.level !== null && bd && this.level !== bd.level) {
      const levels = d.levels
        .filter((l) => l.complete && l.level < bd.level && l.level >= this.level!)
        .sort((a, b) => b.level - a.level);
      for (const lv of levels) {
        const current = lv.level === this.level;
        for (const t of wantedTiles(d, lv, view, viewport)) draw.push({ lv, t, fetch: current });
      }
    }

    const wanted = new Set<string>();
    for (const { lv, t, fetch } of draw) {
      const url = tileUrl(d, lv, t.col, t.row);
      if (this.lru.has(url)) {
        wanted.add(url);
        this.show(url, lv.level, t);
      } else if (fetch) {
        wanted.add(url);
        this.request(url, 0);
      }
    }
    for (const url of this.sink.urls()) if (!wanted.has(url)) this.sink.hide(url);
    this.pinned = wanted;
    this.sink.draw();
    this.onChange(this.stats());
  }

  private pinned: ReadonlySet<string> = new Set();

  private show(url: string, level: number, t: TileRect): void {
    const bitmap = this.lru.get(url);
    if (bitmap) this.sink.show(url, level, t, bitmap);
  }

  private request(url: string, attempt: number): void {
    if (this.inflight.has(url) || this.failed.has(url)) return;
    this.inflight.add(url);
    const gen = this.generation;
    const stale = () => this.destroyed || gen !== this.generation;
    this.fetcher(url).then(
      (bitmap) => {
        if (stale()) return bitmap.close?.();
        this.inflight.delete(url);
        this.lru.set(url, bitmap, bitmap.width * bitmap.height * 4, this.pinned);
        this.refresh();
      },
      () => {
        if (stale()) return;
        this.inflight.delete(url);
        const delay = RETRY_DELAYS_MS[attempt];
        if (delay === undefined) {
          this.failed.add(url);
          this.onChange(this.stats());
          return;
        }
        const timer = setTimeout(() => {
          this.timers.delete(timer);
          this.request(url, attempt + 1);
        }, delay);
        this.timers.add(timer);
      },
    );
  }

  private dispose(url: string, bitmap: ImageBitmap): void {
    this.sink.hide(url);
    bitmap.close?.();
  }
}
