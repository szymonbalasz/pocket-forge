// Date labels shared by features (no feature imports another's internals).

/** The day only, in History's format ("8 Nov"; walk 2 N4, walk REF R7). */
export function dateLabel(at: string): string {
  const d = new Date(at);
  return Number.isFinite(d.getTime())
    ? d.toLocaleDateString([], { month: 'short', day: 'numeric' })
    : '';
}
