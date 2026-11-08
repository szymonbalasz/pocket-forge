// BL-27 (walkthrough D6): error text the UI shows starts with a capital letter.
import { expect, test } from 'vitest';

import { ApiError, sentence } from './index';

test('BL-27 core error messages arrive in sentence case', () => {
  const e = new ApiError({
    code: 'exists',
    message: 'a project named "Crypt" already exists here',
  });
  expect(e.message).toBe('A project named "Crypt" already exists here');
  expect(e.code).toBe('exists');
});

test('BL-27 sentence() changes only a plain leading word', () => {
  expect(sentence('provider error')).toBe('Provider error');
  expect(sentence("can't read it")).toBe("Can't read it");
  expect(sentence('locked')).toBe('Locked');
  expect(sentence('Already fine')).toBe('Already fine');
  // File names, identifiers and paths keep their case.
  expect(sentence('pack.json is unreadable')).toBe('pack.json is unreadable');
  expect(sentence('model_id is unknown')).toBe('model_id is unknown');
  expect(sentence('not_found')).toBe('not_found');
  expect(sentence('')).toBe('');
});
