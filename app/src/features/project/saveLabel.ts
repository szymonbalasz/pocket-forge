import type { ReadOnlyReason } from '../../api';
import type { SaveStatus } from '../../store/project';

const READ_ONLY: Record<ReadOnlyReason, string> = {
  newer_version: 'Read-only (made by a newer Pocket Forge)',
  original_changed: 'Read-only (original changed)',
  original_missing: 'Read-only (original missing)',
};

/** Why a save failed, by `project:save-failed` reason / error code. */
const FAILED: Record<string, string> = {
  locked: 'file locked',
  folder_moved: 'project folder moved',
  io: 'disk error',
  not_found: 'file missing',
};

/** `HH:MM` of an ISO time, or '' when unknown. */
function timeOf(at: string): string {
  const d = new Date(at);
  return at && Number.isFinite(d.getTime())
    ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : '';
}

/** The PRJ status-bar segment's state name (`data-state`). */
export function saveState(save: SaveStatus, readOnly: ReadOnlyReason | null): string {
  if (readOnly) return 'read_only';
  return save.kind;
}

/** Status bar text (asset-project §7): Saved · time / Saving… / Save failed, file locked,
 *  retrying / Read-only (reason) / Conflict. */
export function saveLabel(save: SaveStatus, readOnly: ReadOnlyReason | null): string {
  if (readOnly) return READ_ONLY[readOnly] ?? 'Read-only';
  switch (save.kind) {
    case 'saving':
      return 'Saving…';
    case 'saved':
    case 'unchanged': {
      const t = timeOf(save.at);
      const base = t ? `Saved · ${t}` : 'Saved';
      return save.kind === 'unchanged' ? `${base} · no changes` : base;
    }
    case 'failed': {
      // From the code only: core messages can carry OS text and paths (walkthrough D3).
      const why = FAILED[save.code] ?? save.code.replace(/_/g, ' ');
      return `Save failed, ${why}${save.retrying ? ', retrying' : ''}`;
    }
    case 'conflict':
      return 'Conflict';
  }
}
