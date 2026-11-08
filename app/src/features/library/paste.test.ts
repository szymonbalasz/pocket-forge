import { expect, test } from 'vitest';

import { isTextTarget, parsePastedPaths } from './paste';

test('FR-PRJ-03 a pasted file list: Explorer "Copy as path" lines and file:// URIs become paths', () => {
  expect(parsePastedPaths('"C:\\maps\\Crypt.png"\r\n"C:\\maps\\Sewer.webp"\r\n')).toEqual([
    'C:\\maps\\Crypt.png',
    'C:\\maps\\Sewer.webp',
  ]);
  expect(parsePastedPaths('file:///D:/art/My%20Map.png')).toEqual(['D:/art/My Map.png']);
  expect(parsePastedPaths('/srv/maps/a.png\n\\\\nas\\share\\b.jpg')).toEqual([
    '/srv/maps/a.png',
    '\\\\nas\\share\\b.jpg',
  ]);
});

test('FR-PRJ-03 ordinary pasted text is not a file list', () => {
  expect(parsePastedPaths('a goblin with a lantern')).toEqual([]);
  expect(parsePastedPaths('C:/maps/a.png\nnot a path')).toEqual([]);
  expect(parsePastedPaths('')).toEqual([]);
});

test('FR-PRJ-03 paste in a text field stays in the field', () => {
  const input = document.createElement('input');
  const button = document.createElement('button');
  const area = document.createElement('textarea');
  expect(isTextTarget(input)).toBe(true);
  expect(isTextTarget(area)).toBe(true);
  expect(isTextTarget(button)).toBe(false);
  expect(isTextTarget(document.body)).toBe(false);
});
