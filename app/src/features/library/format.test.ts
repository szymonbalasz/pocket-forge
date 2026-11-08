import { expect, test } from 'vitest';

import { editedLabel, iterationsLabel } from './format';

test('FR-PRJ-49 card meta: iteration count and last edited', () => {
  expect(iterationsLabel(0)).toBe('No iterations');
  expect(iterationsLabel(1)).toBe('1 iteration');
  expect(iterationsLabel(12)).toBe('12 iterations');
  const now = Date.parse('2026-11-08T12:00:00Z');
  expect(editedLabel('2026-11-08T11:59:30Z', now)).toBe('Just now');
  expect(editedLabel('2026-11-08T11:45:00Z', now)).toBe('15 min ago');
  expect(editedLabel('2026-11-08T09:00:00Z', now)).toBe('3 h ago');
  expect(editedLabel('2026-11-07T09:00:00Z', now)).toBe('Yesterday');
  expect(editedLabel('2026-10-01T09:00:00Z', now)).toBe(
    new Date('2026-10-01T09:00:00Z').toLocaleDateString(),
  );
  expect(editedLabel('garbage', now)).toBe('');
  expect(editedLabel(null, now)).toBe('');
});
