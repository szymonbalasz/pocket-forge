// Tile build events seen so far, per pyramid key (FR-EDT-07, D1 of the asset-project walk).
// The core can build a small pyramid and report every level before `edt_tiles_open` returns, and
// before the tile manager exists. The canvas subscribes first and records every event here, so a
// descriptor or manager created later still learns what was built or what failed.
import type { CoreErrorShape, TilesDescriptor } from '../../api';

export class TileEventLog {
  private readonly built = new Map<string, Set<number>>();
  private readonly failed = new Map<string, CoreErrorShape>();

  progress(key: string, level: number): void {
    let s = this.built.get(key);
    if (!s) this.built.set(key, (s = new Set()));
    s.add(level);
  }

  /** Records a failure; a bare message gets the code `error`. Returns it as `{code, message}`
   *  (D6: the code picks the wording and whether Rebuild can help). */
  fail(key: string, error: CoreErrorShape | string): CoreErrorShape {
    const e =
      typeof error === 'string'
        ? { code: 'error', message: error }
        : { code: error.code, message: error.message };
    this.failed.set(key, e);
    return e;
  }

  /** Levels reported built for `key`. */
  levels(key: string): number[] {
    return [...(this.built.get(key) ?? [])];
  }

  failure(key: string): CoreErrorShape | null {
    return this.failed.get(key) ?? null;
  }

  /** `d` with every level reported built marked complete. */
  apply(d: TilesDescriptor): TilesDescriptor {
    const s = this.built.get(d.key);
    if (!s?.size) return d;
    return { ...d, levels: d.levels.map((l) => (s.has(l.level) ? { ...l, complete: true } : l)) };
  }

  /** Before a rebuild (same key, levels built again). */
  clear(): void {
    this.built.clear();
    this.failed.clear();
  }
}
