// A mocked core for Vitest (jsdom): installs `window.__TAURI_INTERNALS__`, the layer under
// app/src/api/, so the real api wrapper and @tauri-apps/api run — as e2e/mock-core.ts does for
// Playwright. Installed once per test file; tests swap the handlers with `useHandlers`.
type Rec = Record<string, unknown>;
type Handler = (input: Rec) => unknown;

export interface TestCore {
  calls: { cmd: string; input: Rec }[];
  /** Replaces the command handlers (and clears `calls`). */
  useHandlers(handlers: Record<string, Handler>): void;
  /** Emits a core event to the UI's listeners. */
  emit(event: string, payload: unknown): void;
  /** Names of the commands called so far. */
  names(): string[];
}

export function installTestCore(): TestCore {
  const w = window as unknown as Rec;
  if (w.__pfVitestCore) return w.__pfVitestCore as TestCore;

  const callbacks = new Map<number, (payload: unknown) => void>();
  const listeners = new Map<number, { event: string; handler: number }>();
  let nextId = 1;
  let handlers: Record<string, Handler> = {};

  const core: TestCore = {
    calls: [],
    useHandlers(h) {
      handlers = h;
      core.calls.length = 0;
    },
    emit(event, payload) {
      for (const [id, l] of listeners)
        if (l.event === event) callbacks.get(l.handler)?.({ event, id, payload });
    },
    names: () => core.calls.map((c) => c.cmd),
  };

  w.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: 'main' },
      currentWebview: { windowLabel: 'main', label: 'main' },
    },
    transformCallback(cb: (p: unknown) => void) {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc: (p: string) => p,
    async invoke(cmd: string, args: Rec) {
      if (cmd === 'plugin:event|listen') {
        const id = nextId++;
        listeners.set(id, { event: String(args.event), handler: Number(args.handler) });
        return id;
      }
      if (cmd === 'plugin:event|unlisten') {
        listeners.delete(Number(args.eventId));
        return null;
      }
      const input = ((args?.input as Rec) ?? args ?? {}) as Rec;
      core.calls.push({ cmd, input });
      const h = handlers[cmd];
      if (!h) throw { code: 'not_found', message: `test core: unknown command ${cmd}` };
      return await h(input);
    },
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
  w.__pfVitestCore = core;
  return core;
}
