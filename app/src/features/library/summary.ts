import type { ImportItem } from '../../api';

/** "2 imported · 1 already in library · 1 skipped (unsupported)". */
export function summarise(items: ImportItem[]): string {
  const imported = items.filter((i) => i.status === 'imported').length;
  const already = items.filter((i) => i.status === 'already_in_library').length;
  const reasons = items.flatMap((i) => (i.status === 'skipped' ? [i.reason] : []));
  const parts = [];
  if (imported) parts.push(`${imported} imported`);
  if (already) parts.push(`${already} already in library`);
  if (reasons.length) {
    const byReason = [...new Set(reasons)].map((r) => r.replace(/_/g, ' ')).join(', ');
    parts.push(`${reasons.length} skipped (${byReason})`);
  }
  return parts.join(' · ') || 'Nothing to import';
}
