import { describe, expect, test } from 'vitest';

import type { TilesDescriptor } from '../../api';
import { TileEventLog } from './tileEvents';

const desc = (key: string): TilesDescriptor => ({
  key,
  base_url: `http://pf-tiles.localhost/t/${key}`,
  width: 1024,
  height: 1280,
  tile_size: 512,
  levels: [0, 1, 2].map((level) => ({
    level,
    scale: 1 / 2 ** level,
    cols: 1,
    rows: 1,
    ext: level ? ('jpg' as const) : ('webp' as const),
    complete: false,
  })),
});

describe('TileEventLog (D1: events before the descriptor)', () => {
  test('FR-EDT-07 levels reported before the descriptor arrives are applied to it', () => {
    const log = new TileEventLog();
    log.progress('k', 2);
    log.progress('k', 1);
    log.progress('other', 0);
    const d = log.apply(desc('k'));
    expect(d.levels.map((l) => l.complete)).toEqual([false, true, true]);
    expect(log.levels('k').sort()).toEqual([1, 2]);
  });

  test('FR-EDT-07 a failure before the descriptor is kept for its key', () => {
    const log = new TileEventLog();
    expect(log.fail('k', { code: 'corrupt', message: 'bad body' })).toBe('bad body');
    expect(log.failure('k')).toBe('bad body');
    expect(log.failure('other')).toBeNull();
    log.clear();
    expect(log.failure('k')).toBeNull();
    expect(log.apply(desc('k')).levels.every((l) => !l.complete)).toBe(true);
  });
});
