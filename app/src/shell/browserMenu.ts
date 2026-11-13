// No browser menu (owner decision, 2026-11-13): WebView2's own right-click menu (Back, Refresh,
// Save as, Print, Inspect) never shows. A right-click does what our menus do (#63) and nothing
// elsewhere, except in editable fields, which keep the native menu for Copy and Paste.
//
// The listener is on the document in the bubble phase, so it runs after React's root listener:
// a Radix menu trigger handles (and prevents) the event first, and this only cancels what no one
// handled.

/** An `input`, `textarea` or content-editable element (or inside one). */
function editable(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest('input, textarea, [contenteditable]:not([contenteditable="false"])'))
    return true;
  return target instanceof HTMLElement && target.isContentEditable;
}

function onContextMenu(e: MouseEvent): void {
  if (e.defaultPrevented || editable(e.target)) return;
  e.preventDefault();
}

/** Installs the listener; returns its removal. */
export function installBrowserMenuBlock(doc: Document = document): () => void {
  doc.addEventListener('contextmenu', onContextMenu);
  return () => doc.removeEventListener('contextmenu', onContextMenu);
}
