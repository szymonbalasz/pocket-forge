import { expect, test } from 'vitest';

import { saveLabel, saveState } from './saveLabel';

const at = '2026-11-08T12:34:00.000Z';
const hhmm = new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

test('FR-PRJ-18 the status segment shows Saved · time after a commit, Saving… during one', () => {
  expect(saveLabel({ kind: 'saved', at }, null)).toBe(`Saved · ${hhmm}`);
  expect(saveLabel({ kind: 'saved', at: '' }, null)).toBe('Saved');
  expect(saveLabel({ kind: 'unchanged', at }, null)).toBe(`Saved · ${hhmm} · no changes`);
  expect(saveLabel({ kind: 'saving' }, null)).toBe('Saving…');
  expect(saveState({ kind: 'saving' }, null)).toBe('saving');
});

test('FR-PRJ-21 a locked file shows Save failed, file locked, retrying', () => {
  const failed = { kind: 'failed', code: 'locked', message: 'locked', retrying: true } as const;
  expect(saveLabel(failed, null)).toBe('Save failed, file locked, retrying');
  expect(saveLabel({ ...failed, retrying: false }, null)).toBe('Save failed, file locked');
});

test('FR-PRJ-21 walkthrough D3: a moved folder and other failures show a reason, never the raw message', () => {
  const raw = 'reading C:/x/Maps/hall/.pforge/revisions/r.json: (os error 3)';
  const failed = { kind: 'failed', code: 'folder_moved', message: raw, retrying: true } as const;
  expect(saveLabel(failed, null)).toBe('Save failed, project folder moved, retrying');
  expect(saveLabel({ ...failed, code: 'not_found', retrying: false }, null)).toBe(
    'Save failed, file missing',
  );
  expect(saveLabel({ ...failed, code: 'disk_full', retrying: false }, null)).toBe(
    'Save failed, disk full',
  );
  for (const code of ['folder_moved', 'not_found', 'io', 'x'])
    expect(saveLabel({ ...failed, code }, null)).not.toContain('os error');
});

test('FR-PRJ-26 FR-PRJ-15 FR-PRJ-41 read-only shows its reason; FR-PRJ-35 a conflict shows Conflict', () => {
  expect(saveLabel({ kind: 'saved', at }, 'newer_version')).toBe(
    'Read-only (made by a newer Pocket Forge)',
  );
  expect(saveLabel({ kind: 'saved', at }, 'original_changed')).toBe('Read-only (original changed)');
  expect(saveState({ kind: 'saved', at }, 'original_changed')).toBe('read_only');
  // R84: a changed picked base image, as FR-PRJ-15.
  expect(saveLabel({ kind: 'saved', at }, 'base_changed')).toBe('Read-only (base image changed)');
  expect(saveState({ kind: 'saved', at }, 'base_changed')).toBe('read_only');
  expect(saveLabel({ kind: 'conflict' }, null)).toBe('Conflict');
});
