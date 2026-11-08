// EDT e2e helpers on top of mock-core: open the fixture map, read the `__pfTest` hook, override
// EDT command handlers per test (window.__pfMockHandlers).
import { expect, type Page } from '@playwright/test';

import { defaultSeed, mockCore, openFirstCard, type MockSeed } from './mock-core';

export interface EdtMockOptions {
  /** Levels not built yet at open (default: none). */
  incomplete?: number[];
  /** `export_get_settings` values; omitted → the command fails (EXP not present). */
  pxPerSquare?: { source: number | null; target: number | null };
  /** The project opens read-only for this reason (FR-EDT-29, BL-10). */
  readOnly?: 'newer_version' | 'original_changed' | 'original_missing';
  seed?: MockSeed;
  /** Like a small real pyramid (D1): nothing built at open, and the whole build is reported
   *  (every level, then ready or failed) before `edt_tiles_open` returns. */
  buildOnOpen?: 'ready' | 'failed';
}

/** The mocked map: 2048×2560 → levels 0–3 (4×5, 2×3, 1×2, 1×1 tiles). Every tile is the fixture
 *  PNG, stretched; tests assert on state, not pixels. */
export const MAP = { w: 2048, h: 2560 };

export async function openMap(page: Page, opts: EdtMockOptions = {}): Promise<void> {
  await installMap(page, opts);
  await page.goto('/');
  await openFirstCard(page);
  if (opts.buildOnOpen === 'failed') return;
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  await page.waitForFunction(() => !!window.__pfTest?.view);
}

/** The mocks `openMap` installs, without opening anything. */
export async function installMap(page: Page, opts: EdtMockOptions = {}): Promise<void> {
  const base = opts.seed ?? { ...defaultSeed, image: MAP };
  const seed = opts.readOnly
    ? { ...base, projects: base.projects.map((p) => ({ ...p, read_only: opts.readOnly! })) }
    : base;
  await mockCore(page, seed);
  await page.addInitScript(
    (o) => {
      type Rec = Record<string, unknown>;
      const h = ((window as unknown as Rec).__pfMockHandlers ??= {}) as Record<
        string,
        (input: Rec) => unknown
      >;
      const calls = ((window as unknown as Rec).__pfEdtTilesOpen = []) as Rec[];
      h.edt_tiles_open = (input) => {
        calls.push(input);
        const rebuilt = input.rebuild === true;
        const key = rebuilt ? 'tk2' : 'tk1';
        const { w, h: ht } = o.size;
        const levels = [];
        for (let level = 0; ; level++) {
          const lw = Math.ceil(w / 2 ** level);
          const lh = Math.ceil(ht / 2 ** level);
          const cols = Math.ceil(lw / 512);
          const rows = Math.ceil(lh / 512);
          const complete = rebuilt || (!o.buildOnOpen && !(o.incomplete ?? []).includes(level));
          levels.push({
            level,
            scale: 1 / 2 ** level,
            cols,
            rows,
            ext: level ? 'jpg' : 'webp',
            complete,
          });
          if (cols === 1 && rows === 1) break;
        }
        if (o.buildOnOpen && !rebuilt) {
          const m = (window as unknown as { __pfMock: { emit: (e: string, p: unknown) => void } })
            .__pfMock;
          if (o.buildOnOpen === 'failed') {
            // A failure Rebuild can fix (D6: `corrupt` / `not_found` offer no Rebuild).
            const error = { code: 'io', message: 'image body is unreadable' };
            m.emit('edt:tiles-failed', { key, error });
          } else {
            for (let i = levels.length - 1; i >= 0; i--)
              m.emit('edt:tiles-progress', { key, level: i });
            m.emit('edt:tiles-ready', { key });
          }
        }
        return {
          key,
          base_url: `http://pf-tiles.localhost/t/${key}`,
          width: w,
          height: ht,
          tile_size: 512,
          levels,
        };
      };
      // FR-EDT-18/28: masks as PRJ `put_mask` keeps them; the draft points at the latest one and
      // keeps the region (as RGN's draft save would), so a reopen restores it (FR-EDT-19).
      const masks = ((window as unknown as Rec).__pfEdtMasks ??= {}) as Record<string, Rec>;
      let maskSeq = 0;
      h.edt_mask_submit ??= (input) => {
        const prj = (window as unknown as Rec).__pfPrjMock as {
          projects: Map<string, Rec>;
          openPath: string | null;
        };
        const p = prj.openPath ? prj.projects.get(prj.openPath) : null;
        if (p?.read_only) return Promise.reject({ code: 'read_only', message: 'Read-only' });
        const rect = input.rect as { w: number; h: number };
        if (atob(String(input.data_b64)).length !== rect.w * rect.h)
          return Promise.reject({ code: 'size_mismatch', message: 'size' });
        const id = `01JMASK${String(++maskSeq).padStart(19, '0')}`;
        masks[id] = { rect: input.rect, data_b64: input.data_b64 };
        const w = window as unknown as Rec;
        w.__pfMockSnapshot = {
          ...((w.__pfMockSnapshot as Rec) ?? {}),
          draft: {
            final_prompt: '',
            model_id: null,
            controls: {},
            seed: null,
            region: { inner: input.rect, margin: 128 },
            mask_id: id,
            updated_at: '2026-11-08T12:00:00.000Z',
          },
        };
        (w.__pfMock as { emit: (e: string, p: unknown) => void } | undefined)?.emit(
          'project:committed',
          { seq: maskSeq },
        );
        return { mask_id: id, rect: input.rect };
      };
      h.edt_mask_read ??= (input) => {
        const m = masks[String(input.mask_id)];
        if (!m) return Promise.reject({ code: 'unknown_id', message: 'unknown mask' });
        return { mask_id: input.mask_id, ...m };
      };
      if (o.pxPerSquare) {
        h.export_get_settings = () => ({
          source_px_per_square: o.pxPerSquare!.source,
          target_px_per_square: o.pxPerSquare!.target,
        });
      }
    },
    { ...opts, size: seed.image },
  );
}

/** Base64 of mask bytes (as the core sends them). */
export const b64 = (bytes: number[]): string =>
  Buffer.from(Uint8Array.from(bytes)).toString('base64');

/** Calls a `window.__pfTest` entry. */
export function hook<T = unknown>(page: Page, name: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(([n, a]) => window.__pfTest![n as string]!(...(a as unknown[])) as T, [
    name,
    args,
  ] as const);
}

export type View = { scale: number; tx: number; ty: number };
export const view = (page: Page) => hook<View>(page, 'view');

export async function stageBox(page: Page) {
  const b = await page.getByTestId('edt-canvas-stage').boundingBox();
  if (!b) throw new Error('no stage');
  return b;
}
