// The iteration tracker's logic (FR-RGN-40): row order, the default selection, which row is the
// job strip, the job actions per state (FR-RGN-19/20/21) and the picker's candidates. Pure.
import type { CandidateRecord, JobState, RgnIteration } from '../../api';

const TERMINAL: JobState[] = ['failed', 'canceled', 'lost', 'closed'];
export const isTerminal = (s: JobState): boolean => TERMINAL.includes(s);
/** Running: submit is disabled and the job strip shows progress. */
export const isActive = (s: JobState): boolean => !isTerminal(s) && s !== 'fetched';

/** Newest first: the active job without candidates (n = null) leads, then by `n` descending. */
export function orderRows(rows: RgnIteration[]): RgnIteration[] {
  return [...rows].sort((a, b) => {
    if (a.n === null && b.n !== null) return -1;
    if (b.n === null && a.n !== null) return 1;
    return (b.n ?? 0) - (a.n ?? 0);
  });
}

/** The newest row is the job strip unless its job is closed (accepted or dismissed). */
export const stripJobId = (rows: RgnIteration[]): string | null => {
  const first = orderRows(rows)[0];
  return first && first.state !== 'closed' ? first.job_id : null;
};

/** Keeps the current selection while it exists; else the newest row. */
export function defaultSelection(rows: RgnIteration[], current: string | null): string | null {
  if (current && rows.some((r) => r.job_id === current)) return current;
  return orderRows(rows)[0]?.job_id ?? null;
}

/** FR-RGN-19: cancel in submitted, queued, running. */
export const canCancel = (s: JobState): boolean =>
  s === 'submitted' || s === 'queued' || s === 'running';
/** FR-RGN-21: retry pre-fills the panel from any job that has ended or returned. */
export const canRetry = (s: JobState): boolean => isTerminal(s) || s === 'fetched';
/** FR-RGN-28: dismiss a returned or ended job that is not closed yet. */
export const canDismiss = (s: JobState): boolean =>
  s === 'fetched' || s === 'failed' || s === 'canceled' || s === 'lost';

export interface TrackerCandidate {
  id: string;
  index: number;
  status: 'ready' | 'invalid';
  starred: boolean;
  accepted: boolean;
  /** FR-RGN-24: the aspect differs from the sent one by > 1 % (resized to the outer rect). */
  aspectWarning: boolean;
}

/** FR-RGN-40: the iteration's non-trashed candidates, in provider order (index 1-based). */
export function iterationCandidates(all: CandidateRecord[], jobId: string): TrackerCandidate[] {
  return all
    .filter((c) => c.job_id === jobId && c.trash_id === null && c.status !== 'trashed')
    .sort((a, b) => a.index - b.index)
    .map((c) => ({
      id: c.id,
      index: c.index + 1,
      status: c.status === 'invalid' ? 'invalid' : 'ready',
      starred: c.starred,
      accepted: c.status === 'accepted',
      aspectWarning: c.aspect_warning === true,
    }));
}

/** §7 confirm dialog: unstarred `new` candidates of the previous (fetched) job, which the next
 *  submit trashes (FR-RGN-28). */
export function trashOnSubmit(rows: RgnIteration[], all: CandidateRecord[]): number {
  const first = orderRows(rows)[0];
  if (!first || first.state !== 'fetched') return 0;
  return all.filter(
    (c) => c.job_id === first.job_id && c.status === 'new' && !c.starred && c.trash_id === null,
  ).length;
}

/** FR-RGN-45: the `N refs` chip tooltip, one line per file sent: name, "project" or its pack,
 *  and its role when it has one. */
export function refsTooltip(row: RgnIteration): string {
  return (row.reference_files ?? [])
    .map((f) => [f.file, f.pack_name ?? 'project', f.role].filter(Boolean).join(' · '))
    .join('\n');
}
