// "Open the settings" across features: the section rail's Settings entry (PRJ) asks, the
// provider feature answers with its settings dialog. Features never import each other's
// internals, so they meet here.
type Listener = () => void;
const listeners = new Set<Listener>();

/** Asks whoever shows the settings to open them. */
export function requestSettings(): void {
  for (const l of listeners) l();
}

/** Subscribes to settings requests. Returns the unsubscribe function. */
export function onSettingsRequested(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
