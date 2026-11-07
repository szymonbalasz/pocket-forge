import { expect, test } from 'vitest';

import { summarise } from './summary';

test('FR-PRJ-06 the import summary lists imported, already in library and skipped with reasons', () => {
  expect(
    summarise([
      { status: 'imported', source: 'a', path: 'A' },
      { status: 'imported', source: 'b', path: 'B' },
      { status: 'already_in_library', source: 'c', path: 'C' },
      { status: 'skipped', source: 'd', reason: 'unsupported' },
      { status: 'skipped', source: 'e', reason: 'too_large' },
    ]),
  ).toBe('2 imported · 1 already in library · 2 skipped (unsupported, too large)');
  expect(summarise([])).toBe('Nothing to import');
});
