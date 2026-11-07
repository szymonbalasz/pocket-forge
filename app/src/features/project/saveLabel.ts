import type { SaveStatus } from '../../store/project';

const READ_ONLY: Record<string, string> = {
  newer_version: 'Read-only (made by a newer Pocket Forge)',
  original_changed: 'Read-only (original changed)',
  original_missing: 'Read-only (original missing)',
};

/** Status bar text (asset-project §7). */
export function saveLabel(save: SaveStatus, readOnly: string | null): string {
  if (readOnly) return READ_ONLY[readOnly] ?? 'Read-only';
  switch (save.kind) {
    case 'unsaved':
      return 'Not saved yet';
    case 'saving':
      return 'Saving…';
    case 'saved':
      return save.at ? `Saved · ${new Date(save.at).toLocaleTimeString()}` : 'Saved';
    case 'unchanged':
      return 'Saved · no changes';
    case 'failed':
      return save.code === 'locked'
        ? 'Save failed, file locked, retrying'
        : `Save failed (${save.code})`;
  }
}
