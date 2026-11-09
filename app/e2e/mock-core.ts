// A mocked core for Playwright (KQ-33). It replaces window.__TAURI_INTERNALS__, the layer under
// app/src/api/, so the real api wrapper runs. Tests seed it with `MockSeed` and read the calls
// back through window.__pfMock. PRJ commands come from the shared mock in src/api/mocks/prj.ts
// (the same one Vitest uses), injected with `fn.toString()`.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import type { Page } from '@playwright/test';

import { prjMock, type PrjMockProject, type PrjMockSeed } from '../src/api/mocks/prj';
import { installPrmMock } from '../src/api/mocks/prm';

export const FIXTURE = fileURLToPath(
  new URL('../../fixtures/images/grid-map-512x640.png', import.meta.url),
);
export const FIXTURE_SIZE = { w: 512, h: 640 };
export const LIBRARY = 'C:/pf-e2e/library';

export interface MockSeed {
  configured: boolean;
  /** Projects in the library (the home grid's cards). */
  projects: PrjMockProject[];
  image: { w: number; h: number };
  /** Extra PRJ mock options (folder contents for drops, creation delay, unavailable root). */
  prj?: Partial<Pick<PrjMockSeed, 'dirs' | 'createDelayMs' | 'unavailable' | 'now'>>;
}

export const defaultSeed: MockSeed = {
  configured: true,
  projects: [
    {
      section: 'maps',
      name: 'grid-map',
      original: 'grid-map-512x640.png',
      iterations: 0,
      last_edited: '2026-11-07T10:00:00.000Z',
    },
  ],
  image: FIXTURE_SIZE,
};

/** The project folder of a seeded project. */
export const projectPath = (section: 'maps' | 'tokens' | 'items', name: string) =>
  `${LIBRARY}/${{ maps: 'Maps', tokens: 'Tokens', items: 'Items' }[section]}/${name}`;

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
  const prjSeed: PrjMockSeed = {
    library: LIBRARY,
    configured: seed.configured,
    image: seed.image,
    projects: seed.projects,
    now: '2026-11-08T12:00:00.000Z',
    ...seed.prj,
  };
  // PRJ handlers never replace one a feature mock registered already (e.g. rgn-mock installs
  // before mockCore); feature mocks added later overwrite them as usual.
  await page.addInitScript({
    content: `(() => {
      const w = window;
      const m = (${prjMock.toString()})(${JSON.stringify(prjSeed)}, {
        emit: (e, p) => w.__pfMock && w.__pfMock.emit(e, p),
        snapshotOverride: () => w.__pfMockSnapshot,
      });
      w.__pfPrjMock = m.state;
      const h = (w.__pfMockHandlers ??= {});
      for (const [k, v] of Object.entries(m.handlers)) if (!(k in h)) h[k] = v;
    })();`,
  });
  // PRM helper commands (the Settings section and the prompt block, prompt-helper §9).
  await page.addInitScript({ content: `(${installPrmMock.toString()})({});` });
  await page.addInitScript((image: { w: number; h: number }) => {
    type Rec = Record<string, unknown>;
    const calls: { cmd: string; args: unknown }[] = [];
    const callbacks = new Map<number, (payload: unknown) => void>();
    let nextId = 1;
    const w = window as unknown as Rec;
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
      edt_image_open: () => ({
        key: 'k1',
        url: 'http://pf-tiles.localhost/i/k1.png',
        width: image.w,
        height: image.h,
      }),
      // EDT: a 512 px pyramid of the fixture (512×640 → level 0: 1×2 tiles, level 1: 1 tile).
      edt_tiles_open: () => ({
        key: 'tk1',
        base_url: 'http://pf-tiles.localhost/t/tk1',
        width: image.w,
        height: image.h,
        tile_size: 512,
        levels: [
          {
            level: 0,
            scale: 1,
            cols: Math.ceil(image.w / 512),
            rows: Math.ceil(image.h / 512),
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
        const { w: iw, h: ih } = image;
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
    /** PRJ (src/api/mocks/prj.ts) and feature mocks (e2e/<feature>-mock.ts) add or override
     *  handlers here. */
    const extra = (w.__pfMockHandlers ??= {}) as Record<string, (input: Rec) => unknown>;

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
        calls.push({ cmd, args });
        const h = cmd.startsWith('plugin:event|') ? handlers[cmd] : (extra[cmd] ?? handlers[cmd]);
        if (!h) return fail('not_found', `mock: unknown command ${cmd}`);
        return h((args?.input as Rec) ?? args ?? {});
      },
    };
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: () => undefined,
    };
    w.__pfMock = {
      calls,
      /** Emits a core event to the UI's listeners. */
      emit(event: string, payload: unknown) {
        for (const [id, l] of listeners)
          if (l.event === event) callbacks.get(l.handler)?.({ event, id, payload });
      },
    };
  }, seed.image);
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

/** A native file drop on the window (Tauri's `tauri://drag-drop`). */
export function dropFiles(page: Page, paths: string[]): Promise<void> {
  return emit(page, 'tauri://drag-drop', { paths, position: { x: 10, y: 10 } });
}

/** Commands the UI called, in order. */
export function calls(page: Page): Promise<{ cmd: string; args: unknown }[]> {
  return page.evaluate(
    () =>
      (window as unknown as { __pfMock: { calls: { cmd: string; args: unknown }[] } }).__pfMock
        .calls,
  );
}

/** Opens the first home-grid card and waits for the canvas. */
export async function openFirstCard(page: Page): Promise<void> {
  await page.getByTestId('prj-home-card').first().getByRole('button').click();
}
