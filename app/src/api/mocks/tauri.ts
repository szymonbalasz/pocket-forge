// A fake `window.__TAURI_INTERNALS__` for Vitest (jsdom), with the same contract as
// `e2e/mock-core.ts`: commands go to `window.__pfMockHandlers`, `project_get` merges
// `window.__pfMockSnapshot` over a base snapshot, events are emitted with `window.__pfMock.emit`.
// The real api wrapper runs on top of it. Test-only; never imported by app code.
import type { ProjectSnapshot } from '../types';

type Rec = Record<string, unknown>;

export interface FakeTauri {
  calls: { cmd: string; input: Rec }[];
  emit(event: string, payload: unknown): void;
  /** Commands called so far with this name. */
  callsOf(cmd: string): Rec[];
  uninstall(): void;
}

export function baseSnapshot(size = { w: 2048, h: 2560 }): ProjectSnapshot {
  return {
    project_id: '01JPROJECT0000000000000000',
    name: 'map.png',
    image_path: 'C:/pf-test/library/Maps/map/working/map.png',
    project_dir: 'C:/pf-test/library/Maps/map',
    saved: true,
    read_only: null,
    warnings: [],
    width: size.w,
    height: size.h,
    format: 'png',
    layers: [],
    draft: null,
    revisions: [],
    head_revision: 'r0',
    commit_seq: 1,
  };
}

export function installFakeTauri(base: ProjectSnapshot = baseSnapshot()): FakeTauri {
  const w = window as unknown as Rec;
  const handlers = (w.__pfMockHandlers = {}) as Record<string, (input: Rec) => unknown>;
  w.__pfMockSnapshot = {};
  const calls: { cmd: string; input: Rec }[] = [];
  const callbacks = new Map<number, (payload: unknown) => void>();
  const listeners = new Map<number, { event: string; handler: number }>();
  let nextId = 1;

  const builtin: Record<string, (input: Rec) => unknown> = {
    project_get: () => ({ ...base, ...(w.__pfMockSnapshot as Rec) }),
    edt_image_open: (input) => ({
      key: `th-${String(input.image_id)}`,
      url: `http://pf-tiles.localhost/i/th-${String(input.image_id)}.png`,
      width: 256,
      height: 256,
    }),
    'plugin:event|listen': (input) => {
      const id = nextId++;
      listeners.set(id, { event: String(input.event), handler: Number(input.handler) });
      return id;
    },
    'plugin:event|unlisten': (input) => {
      listeners.delete(Number(input.eventId));
      return null;
    },
  };

  const emit = (event: string, payload: unknown) => {
    for (const [id, l] of listeners)
      if (l.event === event) callbacks.get(l.handler)?.({ event, id, payload });
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
      const input = ((args?.input as Rec) ?? args ?? {}) as Rec;
      calls.push({ cmd, input });
      const h = handlers[cmd] ?? builtin[cmd];
      if (!h) return Promise.reject({ code: 'not_found', message: `mock: unknown command ${cmd}` });
      return h(input);
    },
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
  w.__pfMock = { calls, emit };

  return {
    calls,
    emit,
    callsOf: (cmd) => calls.filter((c) => c.cmd === cmd).map((c) => c.input),
    uninstall() {
      for (const k of [
        '__TAURI_INTERNALS__',
        '__TAURI_EVENT_PLUGIN_INTERNALS__',
        '__pfMock',
        '__pfMockHandlers',
        '__pfMockSnapshot',
        '__pfRgnMock',
      ])
        delete w[k];
    },
  };
}
