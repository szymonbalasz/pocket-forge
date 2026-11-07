import { describe, expect, test } from 'vitest';

import type { TilesDescriptor } from '../../api';
import { actualScale, displayScale, fit } from './view';
import { backdropLevel, ByteLru, chooseLevel, tileRect, tileUrl, wantedTiles } from './tiles';

/** 4200×5600 (typical, #30) → levels 0..4; level 4 = 263×350 fits one tile. */
function desc(complete = true): TilesDescriptor {
  const levels = [0, 1, 2, 3, 4].map((level) => {
    const w = Math.ceil(4200 / 2 ** level);
    const h = Math.ceil(5600 / 2 ** level);
    return {
      level,
      scale: 1 / 2 ** level,
      cols: Math.ceil(w / 512),
      rows: Math.ceil(h / 512),
      ext: level === 0 ? ('webp' as const) : ('jpg' as const),
      complete,
    };
  });
  return {
    key: 'k',
    base_url: 'http://pf-tiles.localhost/t/k',
    width: 4200,
    height: 5600,
    tile_size: 512,
    levels,
  };
}

describe('tile pyramid maths', () => {
  test('FR-EDT-08 the level is the smallest whose scale is ≥ the device scale (scale × DPR)', () => {
    const { levels } = desc();
    expect(chooseLevel(levels, 1)).toBe(0);
    expect(chooseLevel(levels, 4)).toBe(0);
    expect(chooseLevel(levels, 0.5)).toBe(1);
    expect(chooseLevel(levels, 0.26)).toBe(1);
    expect(chooseLevel(levels, 0.25)).toBe(2);
    expect(chooseLevel(levels, 0.01)).toBe(4);
  });

  test('FR-EDT-05 the level follows DPR: the same CSS scale needs a finer level at DPR 2', () => {
    const { levels } = desc();
    expect(chooseLevel(levels, 0.2 * 1)).toBe(2);
    expect(chooseLevel(levels, 0.2 * 1.5)).toBe(1);
    expect(chooseLevel(levels, 0.2 * 2)).toBe(1);
    expect(chooseLevel(levels, 0.3 * 2)).toBe(0);
  });

  test('FR-EDT-08 only built levels are used; nothing built → null', () => {
    const d = desc(false);
    expect(chooseLevel(d.levels, 1)).toBeNull();
    d.levels[4]!.complete = true;
    expect(chooseLevel(d.levels, 1)).toBe(4);
    d.levels[2]!.complete = true;
    expect(chooseLevel(d.levels, 1)).toBe(2);
    expect(chooseLevel(d.levels, 0.01)).toBe(4);
    expect(backdropLevel(d.levels)?.level).toBe(4);
  });

  test('FR-EDT-07 tile rects cover the image exactly, edge tiles partial', () => {
    const d = desc();
    for (const lv of d.levels) {
      let area = 0;
      for (let r = 0; r < lv.rows; r++)
        for (let c = 0; c < lv.cols; c++) {
          const t = tileRect(d, lv.level, c, r);
          area += t.w * t.h;
          expect(t.x + t.w).toBeLessThanOrEqual(d.width + 1e-6);
          expect(t.y + t.h).toBeLessThanOrEqual(d.height + 1e-6);
        }
      expect(area).toBeCloseTo(d.width * d.height, 3);
    }
    const last = tileRect(d, 0, 8, 10);
    expect([last.x, last.y, last.w, last.h]).toEqual([4096, 5120, 104, 480]);
    expect(tileUrl(d, d.levels[0]!, 8, 10)).toBe('http://pf-tiles.localhost/t/k/0/8_10.webp');
  });

  test('FR-EDT-08 the wanted set is exactly the tiles that intersect the viewport', () => {
    const d = desc();
    const lv = d.levels[0]!;
    const viewport = { w: 1000, h: 600 };
    // 1:1 with image (600, 700) at the viewport's top-left: x 600..1600, y 700..1300.
    const v = { scale: 1, tx: -600, ty: -700 };
    const set = wantedTiles(d, lv, v, viewport).map((t) => `${t.col}_${t.row}`);
    expect(set.sort()).toEqual(['1_1', '1_2', '2_1', '2_2', '3_1', '3_2']);
    expect(wantedTiles(d, lv, { scale: 1, tx: 5000, ty: 0 }, viewport)).toEqual([]);
    const f = fit({ w: d.width, h: d.height }, viewport);
    const l3 = d.levels[3]!;
    expect(wantedTiles(d, l3, f, viewport)).toHaveLength(l3.cols * l3.rows);
  });
});

describe('decoded tile LRU', () => {
  test('FR-EDT-11 evicts the least recently used over budget, never pinned keys', () => {
    const disposed: string[] = [];
    const lru = new ByteLru<string>(100, (v) => disposed.push(v));
    lru.set('a', 'A', 40);
    lru.set('b', 'B', 40);
    lru.get('a'); // b is now the oldest
    lru.set('c', 'C', 40);
    expect(lru.keys()).toEqual(['a', 'c']);
    expect(disposed).toEqual(['B']);
    lru.set('d', 'D', 40, new Set(['a']));
    expect(lru.keys()).toEqual(['a', 'd']);
    expect(lru.bytes).toBe(80);
    // Everything pinned: stays over budget rather than dropping what's on screen.
    lru.set('e', 'E', 90, new Set(['a', 'd']));
    expect(lru.keys()).toEqual(['a', 'd', 'e']);
    lru.clear();
    expect(lru.bytes).toBe(0);
  });
});

describe('presets', () => {
  test('FR-EDT-04 actual = 1 device px per image px; display size = target ÷ source CSS px', () => {
    expect(actualScale(1)).toBe(1);
    expect(actualScale(1.5)).toBeCloseTo(1 / 1.5);
    expect(displayScale(140, 70)).toBe(0.5);
    expect(displayScale(null, 70)).toBeNull();
    expect(displayScale(140, null)).toBeNull();
    expect(displayScale(0, 70)).toBeNull();
  });
});
