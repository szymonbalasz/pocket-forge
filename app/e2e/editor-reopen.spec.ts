// FR-EDT-19: on reopen the draft's mask is loaded, pixel-identical; a different draft region keeps
// the overlap. edt_mask_read is mocked (the real one lands with EDT slice 5).
import { expect, test, type Page } from '@playwright/test';

import { hook, openMap } from './edt-mock';
import { openFirstCard } from './mock-core';

const W = 64;
const H = 32;
const bytes = Array.from({ length: W * H }, (_, i) => (i * 7) % 256);

test('FR-EDT-19 reopen restores the draft mask pixel-identically', async ({ page }) => {
  await page.addInitScript(
    ({ bytes, W, H }) => {
      const w = window as unknown as Record<string, unknown>;
      w.__pfMockSnapshot = {
        draft: {
          final_prompt: '',
          model_id: null,
          controls: {},
          seed: null,
          region: { inner: { x: 200, y: 300, w: W, h: H }, margin: 128 },
          mask_id: '01JMASK000000000000000000',
          updated_at: '2026-11-07T11:00:00.000Z',
        },
      };
      const h = (w.__pfMockHandlers ??= {}) as Record<string, () => unknown>;
      h.edt_mask_read = () => ({ rect: { x: 200, y: 300, w: W, h: H }, bytes });
    },
    { bytes, W, H },
  );
  await openMap(page);
  await expect
    .poll(async () => (await hook<{ inner: unknown } | null>(page, 'region'))?.inner)
    .toEqual({ x: 200, y: 300, w: W, h: H });
  expect(await hook<number[]>(page, 'maskBytes', 0, 0, W, H)).toEqual(bytes);
  // Loading isn't an undo step.
  expect((await hook<{ depth: number }>(page, 'undoDepth')).depth).toBe(0);
});

test('FR-EDT-19 a different draft region keeps the overlap, the rest is 0; a corrupt mask warns', async ({
  page,
}) => {
  await page.addInitScript(
    ({ bytes, W, H }) => {
      const w = window as unknown as Record<string, unknown>;
      w.__pfMockSnapshot = {
        draft: {
          final_prompt: '',
          model_id: null,
          controls: {},
          seed: null,
          region: { inner: { x: 232, y: 300, w: W, h: H }, margin: 128 },
          mask_id: '01JMASK000000000000000000',
          updated_at: '2026-11-07T11:00:00.000Z',
        },
      };
      const h = (w.__pfMockHandlers ??= {}) as Record<string, () => unknown>;
      h.edt_mask_read = () => ({ rect: { x: 200, y: 300, w: W, h: H }, bytes });
    },
    { bytes, W, H },
  );
  await openMap(page);
  await expect
    .poll(async () => (await hook<{ inner: { x: number } } | null>(page, 'region'))?.inner.x)
    .toBe(232);
  const row0 = await hook<number[]>(page, 'maskBytes', 0, 0, W, 1);
  // New x 232..296 vs saved 200..264: the first 32 columns overlap (saved cols 32..63).
  expect(row0.slice(0, 32)).toEqual(bytes.slice(32, 64));
  expect(row0.slice(32)).toEqual(new Array(32).fill(0));
});

test('EDT §11 a corrupt saved mask starts empty with a warning', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__pfMockSnapshot = {
      draft: {
        final_prompt: '',
        model_id: null,
        controls: {},
        seed: null,
        region: { inner: { x: 10, y: 10, w: 40, h: 40 }, margin: 128 },
        mask_id: '01JMASK000000000000000000',
        updated_at: '2026-11-07T11:00:00.000Z',
      },
    };
    const h = (w.__pfMockHandlers ??= {}) as Record<string, () => unknown>;
    h.edt_mask_read = () => Promise.reject({ code: 'corrupt', message: 'not 8-bit grey' });
  });
  await openMap(page);
  await expect(page.getByTestId('edt-mask-warning')).toBeVisible();
  expect((await hook<{ sum: number }>(page, 'mask')).sum).toBe(0);
});

/** Konva nodes of class Image with `name` on the live stage, and whether their layer drew them
 *  (the layer canvas has alpha at the node's centre). A Layer named "mask" doesn't count. */
async function drawnImage(page: Page, name: string) {
  return page.evaluate((name) => {
    const stage = window.__pfTest!.stage!();
    const node = stage.findOne(
      (n: { getClassName: () => string; hasName: (s: string) => boolean }) =>
        n.getClassName() === 'Image' && n.hasName(name),
    );
    if (!node || node.getStage() !== stage || !node.isVisible())
      return { found: !!node, drawn: false };
    const layer = node.getLayer()!;
    const r = node.getClientRect();
    const ratio = layer.getCanvas().getPixelRatio();
    const ctx = layer.getNativeCanvasElement().getContext('2d')!;
    const a = ctx.getImageData(
      Math.round((r.x + r.width / 2) * ratio),
      Math.round((r.y + r.height / 2) * ratio),
      1,
      1,
    ).data[3]!;
    return { found: true, drawn: a > 0 };
  }, name);
}

test('FR-EDT-14 FR-EDT-24 after close and reopen the mask tint and the compare image are drawn', async ({
  page,
}) => {
  await openMap(page);
  await page.getByTestId('prj-panel-close').click();
  await openFirstCard(page);
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  await hook(page, 'setView', { scale: 0.5, tx: 0, ty: 0 });

  // Mask: a region, filled → the tint image is on the stage and drawn.
  await page.keyboard.press('r');
  const b = (await page.getByTestId('edt-canvas-stage').boundingBox())!;
  await page.mouse.move(b.x + 50, b.y + 50);
  await page.mouse.down();
  await page.mouse.move(b.x + 150, b.y + 100, { steps: 8 });
  await page.mouse.up();
  await expect
    .poll(async () => (await hook<{ inner: { w: number } } | null>(page, 'region'))?.inner.w)
    .toBe(200);
  await page.getByTestId('edt-toolbar-fill').click();
  await expect.poll(() => drawnImage(page, 'mask')).toEqual({ found: true, drawn: true });

  // Compare: selecting a candidate draws its after-image.
  const rect = { x: 400, y: 400, w: 600, h: 400 };
  await hook(page, 'setCandidates', [
    {
      id: 'C1',
      index: 1,
      status: 'ready',
      starred: false,
      thumbUrl: 'http://pf-tiles.localhost/i/th1.png',
      preview: { url: 'http://pf-tiles.localhost/i/pv1.png', rect },
    },
  ]);
  await page.keyboard.press('1');
  await expect.poll(() => drawnImage(page, 'compare-after')).toEqual({ found: true, drawn: true });
});
