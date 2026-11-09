import { expect, test } from 'vitest';

import { storageLabel } from './format';

test('R84 FR-PRJ-33 Storage names working/ "Base image" in a project whose bottom image was picked', () => {
  expect(storageLabel('original', { layers: [{ kind: 'base' }] })).toBe('Base image');
  expect(storageLabel('original', { layers: [{ kind: 'original' }, { kind: 'patch' }] })).toBe(
    'Original',
  );
  expect(storageLabel('original', null)).toBe('Original');
  expect(storageLabel('results', { layers: [{ kind: 'base' }] })).toBe('Results');
  expect(storageLabel('mystery', null)).toBe('mystery');
});
