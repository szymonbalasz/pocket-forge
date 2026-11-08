// Pasted file lists (FR-PRJ-03): Explorer's "Copy as path" puts quoted absolute paths on the
// clipboard as text, one per line; `file://` URIs come from other apps.

const ABSOLUTE = /^(?:[A-Za-z]:[\\/]|\\\\|\/)/;

/** The absolute paths in pasted text, or [] when the text is not a file list. */
export function parsePastedPaths(text: string): string[] {
  const lines = text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => l.replace(/^"(.*)"$/, '$1'));
  const paths = lines.map((l) => {
    if (!/^file:\/\//i.test(l)) return l;
    try {
      const p = decodeURIComponent(new URL(l).pathname);
      return /^\/[A-Za-z]:\//.test(p) ? p.slice(1) : p;
    } catch {
      return l;
    }
  });
  return paths.length > 0 && paths.every((p) => ABSOLUTE.test(p)) ? paths : [];
}

/** True while a text field has focus: paste goes to the field, not to project creation. */
export function isTextTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLInputElement && !['button', 'checkbox', 'radio'].includes(target.type))
  );
}
