import type { CreateItem, SkipReason } from '../../api';

const REASON: Record<SkipReason, string> = {
  unsupported: 'unsupported',
  animated: 'animated',
  too_large: 'too large',
  corrupt: 'corrupt',
  not_found: 'not found',
  io: 'read error',
  not_available_offline: 'not available offline',
};

export function reasonLabel(reason: SkipReason): string {
  return REASON[reason] ?? reason.replace(/_/g, ' ');
}

/** "2 created · 1 already in library · 1 skipped (unsupported)" (asset-project §7). */
export function summarise(items: CreateItem[], canceled = false): string {
  const created = items.filter((i) => i.status === 'created').length;
  const already = items.filter((i) => i.status === 'already_in_library').length;
  const reasons = items.flatMap((i) => (i.status === 'skipped' ? [i.reason] : []));
  const parts = [];
  if (created) parts.push(`${created} created`);
  if (already) parts.push(`${already} already in library`);
  if (reasons.length) {
    const byReason = [...new Set(reasons)].map(reasonLabel).join(', ');
    parts.push(`${reasons.length} skipped (${byReason})`);
  }
  if (canceled) parts.push('canceled');
  return parts.join(' · ') || 'No images found';
}

/** The file name of a source path, for the summary list. */
export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}
