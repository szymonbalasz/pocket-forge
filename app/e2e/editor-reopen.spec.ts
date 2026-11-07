// FR-EDT-19: on reopen the draft's mask is loaded, pixel-identical; a different draft region keeps
// the overlap. edt_mask_read is mocked (the real one lands with EDT slice 5).
import { expect, test } from '@playwright/test';

import { hook, openMap } from './edt-mock';

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
