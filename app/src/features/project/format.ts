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
export function dateLabel(at: string): string {
  const d = new Date(at);
  return Number.isFinite(d.getTime())
    ? d.toLocaleDateString([], { month: 'short', day: 'numeric' })
    : '';
}
