// Display formats for the project panel (tabular numbers, ui-guidelines).
import type { RevisionEntry } from '../../api';

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** "0 B", "12 KB", "3.4 MB" (base 1024, one decimal under 10). */
export function bytesLabel(bytes: number): string {
  let v = Math.max(0, bytes);
  let u = 0;
  while (v >= 1024 && u < UNITS.length - 1) {
    v /= 1024;
    u++;
  }
  const n = u === 0 || v >= 10 ? Math.round(v).toString() : v.toFixed(1);
  return `${n} ${UNITS[u]}`;
}

/** A Storage row's label; `working/` holds the picked base image of a generated project. */
export function storageLabel(key: string, project: { layers: { kind: string }[] } | null): string {
  if (key === 'original' && project?.layers[0]?.kind === 'base') return 'Base image';
  return CATEGORY_LABEL[key] ?? key;
}

export const CATEGORY_LABEL: Record<string, string> = {
  original: 'Original',
  layers: 'Layers',
  masks: 'Masks',
  job_inputs: 'Job inputs',
  results: 'Results',
  trash: 'Trash',
  user_folders: 'Your folders',
  exports: 'Exports',
  other: 'Other',
};

const REASON: Record<RevisionEntry['reason'], string> = {
  create: 'Created',
  accept: 'Accepted',
  save: 'Saved',
  restore: 'Restored',
  retune: 'Re-tuned',
  conflict: 'Other machine',
};

/** The label if any, else the reason ("Created", "Accepted", …). */
export function revisionLabel(r: RevisionEntry): string {
  return r.label ? r.label : (REASON[r.reason] ?? r.reason);
}

export function timeLabel(at: string): string {
  const d = new Date(at);
  return Number.isFinite(d.getTime())
    ? d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
    : '';
}

/** The day only, in History's format ("8 Nov"; walk 2 N4). */
export { dateLabel } from '../../shared/dates';

// ------------------------------------------------------------------ drops (FR-PRJ-55, #61)

/** The file name of a dropped path (either separator). */
export function dropName(path: string): string {
  return path.split(/[\/]/).pop() || path;
}

/** A file a drop on a user folder didn't copy, and why. */
export interface DropSkip {
  name: string;
  code: string;
}

const SKIP_REASON: Record<string, string> = {
  not_found: 'not a file',
  not_available_offline: 'not available offline',
  io: 'couldn’t be read',
};

/** "a.png, b.png", or "a.png, b.png, c.png +2 more". */
function names(list: string[]): string {
  const head = list.slice(0, 3).join(', ');
  return list.length > 3 ? `${head} +${list.length - 3} more` : head;
}

/** The notice after a drop on References, Structure or Parts: what was copied in and what was
 *  skipped, with the reason ("Added to References: a.png. Skipped: notes (not a file)."). */
export function folderDropNotice(label: string, added: string[], skipped: DropSkip[]): string {
  const parts: string[] = [];
  if (added.length) parts.push(`Added to ${label}: ${names(added)}.`);
  if (skipped.length) {
    const byReason = new Map<string, string[]>();
    for (const s of skipped) {
      const reason = SKIP_REASON[s.code] ?? s.code.replace(/_/g, ' ');
      byReason.set(reason, [...(byReason.get(reason) ?? []), s.name]);
    }
    const list = [...byReason].map(([reason, n]) => `${names(n)} (${reason})`).join('; ');
    parts.push(`Skipped: ${list}.`);
  }
  return parts.join(' ') || 'Nothing to add.';
}

/** Shown when more than one file is dropped on Working (FR-PRJ-55): nothing changes. */
export const DROP_ONE_IMAGE = 'Drop one image on Working.';

/** Why a drop on Working changed nothing, as a sentence naming the file. */
export function workingDropError(name: string, code: string, message: string): string {
  switch (code) {
    case 'job_active':
      return 'A job is running in this project. Drop the image when it has finished.';
    case 'read_only':
      return 'The project is read-only.';
    case 'unsupported':
      return `${name} isn’t a PNG, JPEG or WebP image.`;
    case 'animated':
      return `${name} is animated; only still images can be used.`;
    case 'too_large':
      return `${name} is too large (more than 16384 px on a side).`;
    case 'corrupt':
      return `${name} can’t be read (the file may be damaged).`;
    case 'not_found':
      return `${name} isn’t a file.`;
    case 'not_available_offline':
      return `${name} isn’t available offline.`;
    case 'folder_moved':
      return 'The project folder was moved or renamed outside Pocket Forge. Reopen it from home.';
    default: {
      const m = message.trim();
      return m ? `${m[0]!.toUpperCase()}${m.slice(1)}${/[.!?]$/.test(m) ? '' : '.'}` : code;
    }
  }
}
