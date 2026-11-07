import { afterEach, describe, expect, test, vi } from 'vitest';

import type { TilesDescriptor } from '../../api';
import { TileManager, type TileSink, type TileStats } from './TileManager';

const desc = (complete: [boolean, boolean]): TilesDescriptor => ({
  key: 'k',
  base_url: 'pf/t/k',
  width: 1000,
  height: 600,
  tile_size: 512,
  levels: [
    { level: 0, scale: 1, cols: 2, rows: 2, ext: 'webp', complete: complete[0] },
    { level: 1, scale: 0.5, cols: 1, rows: 1, ext: 'jpg', complete: complete[1] },
  ],
});

function fakeLayer(): TileSink {
  const shown = new Set<string>();
  return {
    show: (u) => void shown.add(u),
    hide: (u) => void shown.delete(u),
    has: (u) => shown.has(u),
    urls: () => [...shown],
    draw: () => undefined,
    destroy: () => shown.clear(),
  };
}

const bitmap = () => ({ width: 512, height: 512, close: vi.fn() }) as unknown as ImageBitmap;
const flush = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
  vi.useRealTimers();
});

describe('TileManager', () => {
  test('FR-EDT-07 FR-EDT-08 the one-tile level shows first, then the current level once built', async () => {
    const fetched: string[] = [];
    let stats: TileStats | null = null;
    const tm = new TileManager(
      fakeLayer(),
      desc([false, true]),
      (s) => (stats = s),
      async (u) => {
        fetched.push(u);
        return bitmap();
      },
    );
    tm.update({ scale: 1, tx: 0, ty: 0 }, { w: 800, h: 600 }, 1);
    await flush();
    expect(fetched).toEqual(['pf/t/k/1/0_0.jpg']);
    expect(stats!.backdropShown).toBe(true);
    expect(stats!.level).toBe(1);
    tm.markBuilt(0);
    await flush();
    expect(stats!.level).toBe(0);
    expect(fetched.slice(1).sort()).toEqual([
      'pf/t/k/0/0_0.webp',
      'pf/t/k/0/0_1.webp',
      'pf/t/k/0/1_0.webp',
      'pf/t/k/0/1_1.webp',
    ]);
    expect(stats!.shown).toHaveLength(5);
    tm.destroy();
  });

  test('FR-EDT-12 a failing tile retries twice (250 ms, 1 s), then is reported; the backdrop stays', async () => {
    vi.useFakeTimers();
    const attempts: Record<string, number> = {};
    let stats: TileStats | null = null;
    const bad = 'pf/t/k/0/0_0.webp';
    const tm = new TileManager(
      fakeLayer(),
      desc([true, true]),
      (s) => (stats = s),
      async (u) => {
        attempts[u] = (attempts[u] ?? 0) + 1;
        if (u === bad) throw new Error('HTTP 404');
        return bitmap();
      },
    );
    tm.update({ scale: 1, tx: 0, ty: 0 }, { w: 400, h: 400 }, 1);
    await vi.advanceTimersByTimeAsync(0);
    expect(attempts[bad]).toBe(1);
    await vi.advanceTimersByTimeAsync(249);
    expect(attempts[bad]).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(attempts[bad]).toBe(2);
    await vi.advanceTimersByTimeAsync(1000);
    expect(attempts[bad]).toBe(3);
    expect(stats!.failed).toBe(1);
    expect(stats!.backdropShown).toBe(true);
    await vi.advanceTimersByTimeAsync(5000);
    expect(attempts[bad]).toBe(3);
    // Rebuild resets: the tile is fetched again.
    tm.reset(desc([true, true]));
    await vi.advanceTimersByTimeAsync(0);
    expect(attempts[bad]).toBe(4);
    tm.destroy();
  });
});
