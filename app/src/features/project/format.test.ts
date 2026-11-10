import { expect, test } from 'vitest';

import { folderDropNotice, storageLabel, workingDropError } from './format';

test('R84 FR-PRJ-33 Storage names working/ "Base image" in a project whose bottom image was picked', () => {
  expect(storageLabel('original', { layers: [{ kind: 'base' }] })).toBe('Base image');
  expect(storageLabel('original', { layers: [{ kind: 'original' }, { kind: 'patch' }] })).toBe(
    'Original',
  );
  expect(storageLabel('original', null)).toBe('Original');
  expect(storageLabel('results', { layers: [{ kind: 'base' }] })).toBe('Results');
  expect(storageLabel('mystery', null)).toBe('mystery');
});

test('FR-PRJ-55 a folder drop notice names what was added and groups what was skipped by reason', () => {
  expect(folderDropNotice('Parts', ['a.png', 'b.png', 'c.png', 'd.png', 'e.png'], [])).toBe(
    'Added to Parts: a.png, b.png, c.png +2 more.',
  );
  expect(
    folderDropNotice(
      'Structure',
      [],
      [
        { name: 'old', code: 'not_found' },
        { name: 'x.png', code: 'not_available_offline' },
        { name: 'y', code: 'not_found' },
      ],
    ),
  ).toBe('Skipped: old, y (not a file); x.png (not available offline).');
  expect(folderDropNotice('Parts', [], [])).toBe('Nothing to add.');
});

test('FR-PRJ-55 a refused drop on Working says why, naming the file', () => {
  expect(workingDropError('big.png', 'too_large', '')).toBe(
    'big.png is too large (more than 16384 px on a side).',
  );
  expect(workingDropError('a.webp', 'animated', '')).toBe(
    'a.webp is animated; only still images can be used.',
  );
  expect(workingDropError('a.png', 'locked', 'the project file is locked')).toBe(
    'The project file is locked.',
  );
});
