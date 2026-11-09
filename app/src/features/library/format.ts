// Display formats for cards and panels (tabular numbers, ui-guidelines).

/** "3 iterations" / "1 iteration" / "No iterations". */
export function iterationsLabel(n: number): string {
  if (n === 0) return 'No iterations';
  return `${n} iteration${n === 1 ? '' : 's'}`;
}

/** "Just now", "5 min ago", "3 h ago", "Yesterday", else the date; '' when unknown (R45:
 *  an unreadable `project.json` has no `commit.at`). `now` is injectable. */
export function editedLabel(at: string | null, now: number = Date.now()): string {
  if (!at) return '';
  const t = new Date(at).getTime();
  if (!Number.isFinite(t)) return '';
  const s = Math.max(0, (now - t) / 1000);
  if (s < 60) return 'Just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  if (s < 2 * 86400) return 'Yesterday';
  return new Date(t).toLocaleDateString();
}

export const SECTIONS = [
  { id: 'maps', label: 'Maps', noun: 'map' },
  { id: 'tokens', label: 'Tokens', noun: 'token' },
  { id: 'items', label: 'Items', noun: 'item' },
] as const;

export const BADGE_LABEL: Record<string, string> = {
  read_only: 'Read-only',
  conflict: 'Conflict',
  unreadable: 'Unreadable',
  not_available_offline: 'Not available offline',
  no_image: 'No image yet',
};
