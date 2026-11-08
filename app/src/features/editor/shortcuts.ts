// Keyboard shortcuts (ui-guidelines: keyboard first). FR-EDT-30 / R23: single-key and bracket
// shortcuts are ignored while a text input, textarea or contenteditable has focus. #48 (2): Enter
// and Space go to a focused button or other interactive control first.

/** True when the event target (or the focused element) takes text input. */
export function isTextTarget(target: EventTarget | null): boolean {
  const el = (target instanceof Element ? target : null) ?? document.activeElement;
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) {
    const nonText = ['button', 'checkbox', 'radio', 'range', 'color', 'file', 'submit', 'reset'];
    return !nonText.includes(el.type);
  }
  if (el instanceof HTMLSelectElement) return true;
  return el instanceof HTMLElement && el.isContentEditable;
}

const CONTROLS =
  'button, a[href], input, select, textarea, summary, [contenteditable=""], [contenteditable="true"], ' +
  '[role="button"], [role="checkbox"], [role="radio"], [role="switch"], [role="tab"], ' +
  '[role="menuitem"], [role="link"]';

/** True when the target (or the focused element) is a control that Enter or Space activates. */
export function isControlTarget(target: EventTarget | null): boolean {
  const el = (target instanceof Element ? target : null) ?? document.activeElement;
  return !!el && (isTextTarget(el) || !!el.closest(CONTROLS));
}

/** #48 (2): Enter and Space belong to a focused button, link or field, not to a shortcut. */
export function isControlKey(e: KeyboardEvent): boolean {
  return (e.key === 'Enter' || e.key === ' ') && isControlTarget(e.target);
}

/** R41: a mouse press on a button in this container doesn't give the button focus (and drops
 *  focus from wherever it was), so Space-to-pan and tool keys work right after the click. The
 *  click still fires. Keyboard focus (Tab) is unchanged and gets Enter/Space first (#48). */
export function noMouseFocus(e: {
  button: number;
  target: EventTarget;
  preventDefault(): void;
}): void {
  if (e.button !== 0 || !(e.target instanceof Element) || !e.target.closest('button')) return;
  e.preventDefault();
  const active = document.activeElement;
  if (active instanceof HTMLElement && active !== document.body) active.blur();
}

/** A normalised key chord: `Ctrl+Shift+Z`, `B`, `Shift+[`, `Space`, … (Meta counts as Ctrl). */
export function chord(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push('Ctrl');
  if (e.altKey) parts.push('Alt');
  let key = e.key;
  // Shift+[ arrives as `{` on most layouts; use the physical key for brackets.
  if (e.code === 'BracketLeft') key = '[';
  else if (e.code === 'BracketRight') key = ']';
  else if (e.code === 'Backslash') key = '\\';
  else if (key === ' ') key = 'Space';
  else if (key.length === 1) key = key.toUpperCase();
  if (e.shiftKey && key !== 'Shift') parts.push('Shift');
  parts.push(key);
  return parts.join('+');
}

export type ShortcutMap = Record<string, (e: KeyboardEvent) => void>;

/** Runs the handler for the event's chord. Chords without Ctrl/Alt are suppressed in text fields. */
export function dispatchShortcut(e: KeyboardEvent, map: ShortcutMap): boolean {
  const c = chord(e);
  const h = map[c];
  if (!h) return false;
  if (isControlKey(e)) return false;
  const modified = e.ctrlKey || e.metaKey || e.altKey;
  // In a text field: no single keys, and Ctrl+Z / Ctrl+Y stay the field's own undo.
  if (isTextTarget(e.target) && (!modified || /\+[ZY]$/.test(c))) return false;
  e.preventDefault();
  h(e);
  return true;
}
