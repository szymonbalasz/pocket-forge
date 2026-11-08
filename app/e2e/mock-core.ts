// A mocked core for Playwright (KQ-33). It replaces window.__TAURI_INTERNALS__, the layer under
// app/src/api/, so the real api wrapper runs. Tests seed it with `MockSeed` and read the calls
// back through window.__pfMock.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import type { Page } from '@playwright/test';

export const FIXTURE = fileURLToPath(
  new URL('../../fixtures/images/grid-map-512x640.png', import.meta.url),
);
export const FIXTURE_SIZE = { w: 512, h: 640 };
export const LIBRARY = 'C:/pf-e2e/library';

export interface MockSeed {
  configured: boolean;
  recent: { path: string; name: string; opened_at: string }[];
  image: { w: number; h: number };
}

export const defaultSeed: MockSeed = {
  configured: true,
  recent: [
    {
      path: `${LIBRARY}/Imported/grid-map-512x640.png`,
      name: 'grid-map-512x640.png',
      opened_at: '2026-11-07T10:00:00.000Z',
    },
  ],
  image: FIXTURE_SIZE,
};

/** Installs the mock core and serves the fixture on the pf-tiles scheme. */
export async function mockCore(page: Page, seed: MockSeed = defaultSeed): Promise<void> {
  const png = readFileSync(FIXTURE);
  await page.route('http://pf-tiles.localhost/**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'image/png',
      body: png,
      headers: { 'Access-Control-Allow-Origin': '*' },
    }),
  );
  await page.addInitScript(
    ({ seed, library }) => {
      type Rec = Record<string, unknown>;
      const calls: { cmd: string; args: unknown }[] = [];
      const callbacks = new Map<number, (payload: unknown) => void>();
      let nextId = 1;
      let project: Rec | null = null;
      let seq = 0;
      const revisions: Rec[] = [];
      const recent = [...seed.recent];

      const snapshot = (path: string): Rec => ({
        project_id: '01JPROJECT0000000000000000',
        name: path.split('/').pop(),
        image_path: path,
        project_dir: `${path}.pforge`,
        saved: revisions.length > 0,
        read_only: null,
        warnings: [],
        width: seed.image.w,
        height: seed.image.h,
        format: 'png',
        layers: [
          {
            id: '01JLAYER00000000000000000',
            kind: 'original',
            file: null,
            cx: seed.image.w / 2,
            cy: seed.image.h / 2,
            scale: 1,
            rotation: 0,
            opacity: 1,
            visible: true,
          },
        ],
        draft: null,
        revisions: [...revisions],
        head_revision: revisions.at(-1)?.id ?? null,
        commit_seq: seq,
        // Feature mocks override snapshot fields here (layers, read_only, draft, …).
        ...(((window as unknown as Rec).__pfMockSnapshot as Rec | undefined) ?? {}),
      });

      const fail = (code: string, message: string) => Promise.reject({ code, message });

      const handlers: Record<string, (input: Rec) => unknown> = {
        app_info: () => ({
          version: '0.1.0',
          provider_mode: 'fake',
          data_dir: 'C:/pf-e2e/data',
          library_overridden: true,
          warnings: [],
        }),
        app_ready: () => null,
        provider_status: () => ({ mode: 'fake', configured: true, connectivity: 'ok' }),
        library_get_config: () => ({
          roots: seed.configured ? [{ path: library, available: true }] : [],
          import_dest: seed.configured ? `${library}/Imported` : null,
          overridden: false,
          configured: seed.configured,
        }),
        recent_list: () => recent,
        import_paths: (input) => ({
          items: (input.paths as string[]).map((source) =>
            source.endsWith('.psd')
              ? { status: 'skipped', source, reason: 'unsupported' }
              : {
                  status: 'imported',
                  source,
                  path: `${library}/Imported/${source.split(/[\\/]/).pop()}`,
                },
          ),
        }),
        project_open: (input) => {
          const path = input.path as string;
          project = snapshot(path);
          if (!recent.some((r) => r.path === path))
            recent.unshift({
              path,
              name: String(project.name),
              opened_at: '2026-11-07T11:00:00.000Z',
            });
          return project;
        },
        project_get: () => (project ? snapshot(String(project.image_path)) : null),
        project_close: () => {
          project = null;
          return {};
        },
        project_save_revision: () => {
          if (!project) return fail('no_project', 'no asset project is open');
          if (revisions.length > 0)
            return fail('unchanged', 'nothing changed since the last revision');
          seq += 1;
          const rev = {
            id: '01JREV0000000000000000000',
            seq: 0,
            created_at: '2026-11-07T11:00:01.000Z',
            reason: 'create',
          };
          revisions.push(rev);
          return rev;
        },
        edt_image_open: () => ({
          key: 'k1',
          url: 'http://pf-tiles.localhost/i/k1.png',
          width: seed.image.w,
          height: seed.image.h,
        }),
        // EDT: a 512 px pyramid of the fixture (512×640 → level 0: 1×2 tiles, level 1: 1 tile).
        edt_tiles_open: () => ({
          key: 'tk1',
          base_url: 'http://pf-tiles.localhost/t/tk1',
          width: seed.image.w,
          height: seed.image.h,
          tile_size: 512,
          levels: [
            {
              level: 0,
              scale: 1,
              cols: Math.ceil(seed.image.w / 512),
              rows: Math.ceil(seed.image.h / 512),
              ext: 'webp',
              complete: true,
            },
            { level: 1, scale: 0.5, cols: 1, rows: 1, ext: 'jpg', complete: true },
          ],
        }),
        // RGN `rgn_region_fit` without a model (FR-RGN-01/02): snap, clamp, ≥ 16 px, margin
        // (default 128) clamped to the image, outer ≤ 4096. Mirrors pf-core region_edit::fit.
        rgn_region_fit: (input) => {
          const r = input.inner as { x: number; y: number; w: number; h: number };
          const margin = typeof input.margin === 'number' ? input.margin : 128;
          const { w: iw, h: ih } = seed.image;
          const c = (v: number, max: number) => Math.min(max, Math.max(0, Math.round(v)));
          const ax = c(r.x, iw);
          const bx = c(r.x + r.w, iw);
          const ay = c(r.y, ih);
          const by = c(r.y + r.h, ih);
          const [x0, x1] = [Math.min(ax, bx), Math.max(ax, bx)];
          const [y0, y1] = [Math.min(ay, by), Math.max(ay, by)];
          if (x1 <= x0 || y1 <= y0) return fail('out_of_bounds', 'the region is outside the image');
          if (x1 - x0 < 16 || y1 - y0 < 16)
            return fail('region_too_small', 'a region is at least 16 × 16 px');
          const ox0 = Math.max(0, x0 - margin);
          const oy0 = Math.max(0, y0 - margin);
          const outer = {
            x: ox0,
            y: oy0,
            w: Math.min(iw, x1 + margin) - ox0,
            h: Math.min(ih, y1 + margin) - oy0,
          };
          if (outer.w > 4096 || outer.h > 4096)
            return fail('region_too_large', 'the region plus its context margin is over 4096 px');
          return {
            inner: { x: x0, y: y0, w: x1 - x0, h: y1 - y0 },
            outer,
            margin,
            sent_w: outer.w,
            sent_h: outer.h,
            scale: 1,
          };
        },
        'plugin:event|listen': (input) => {
          const id = nextId++;
          const ev = String(input.event);
          listeners.set(id, { event: ev, handler: Number(input.handler) });
          return id;
        },
        'plugin:event|unlisten': (input) => {
          listeners.delete(Number(input.eventId));
          return null;
        },
      };
      const listeners = new Map<number, { event: string; handler: number }>();
      /** Feature mocks (e2e/<feature>-mock.ts) add or override handlers here. */
      const extra = ((window as unknown as Rec).__pfMockHandlers ??= {}) as Record<
        string,
        (input: Rec) => unknown
      >;

      (window as unknown as Rec).__TAURI_INTERNALS__ = {
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
          calls.push({ cmd, args });
          const h = extra[cmd] ?? handlers[cmd];
          if (!h) return fail('not_found', `mock: unknown command ${cmd}`);
          return h((args?.input as Rec) ?? args ?? {});
        },
      };
      (window as unknown as Rec).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => undefined,
      };
      (window as unknown as Rec).__pfMock = {
        calls,
        /** Emits a core event to the UI's listeners. */
        emit(event: string, payload: unknown) {
          for (const [id, l] of listeners)
            if (l.event === event) callbacks.get(l.handler)?.({ event, id, payload });
        },
      };
    },
    { seed, library: LIBRARY },
  );
}

/** Emits a core event (`edt:tiles-progress`, `export:settings-changed`, …) to the UI. */
export function emit(page: Page, event: string, payload: unknown): Promise<void> {
  return page.evaluate(
    ([e, p]) =>
      (window as unknown as { __pfMock: { emit: (e: string, p: unknown) => void } }).__pfMock.emit(
        e as string,
        p,
      ),
    [event, payload] as const,
  );
}

/** Commands the UI called, in order. */
export function calls(page: Page): Promise<{ cmd: string; args: unknown }[]> {
  return page.evaluate(
    () =>
      (window as unknown as { __pfMock: { calls: { cmd: string; args: unknown }[] } }).__pfMock
        .calls,
  );
}
