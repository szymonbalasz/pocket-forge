import { expect, test } from 'vitest';

import { summarise } from './summary';

test('FR-PRJ-06 FR-PRJ-08 the creation summary lists created, already in library and skipped with reasons', () => {
  expect(
    summarise([
      { status: 'created', source: 'a', path: 'A' },
      { status: 'created', source: 'b', path: 'B' },
      { status: 'already_in_library', source: 'c', path: 'C' },
      { status: 'skipped', source: 'd', reason: 'unsupported' },
      { status: 'skipped', source: 'e', reason: 'too_large' },
      { status: 'skipped', source: 'f', reason: 'animated' },
      { status: 'skipped', source: 'g', reason: 'corrupt' },
    ]),
  ).toBe(
    '2 created · 1 already in library · 4 skipped (unsupported, too large, animated, corrupt)',
  );
  expect(summarise([])).toBe('No images found');
});

test('FR-PRJ-11 a canceled creation says so and keeps the finished count', () => {
  expect(summarise([{ status: 'created', source: 'a', path: 'A' }], true)).toBe(
    '1 created · canceled',
  );
});
