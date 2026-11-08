// EDT slice 4: region tool, mask tools and undo (mocked core, including RGN's rgn_region_fit).
import { expect, test, type Page } from '@playwright/test';

import { hook, openMap, stageBox, view } from './edt-mock';

type Region = { inner: { x: number; y: number; w: number; h: number }; outer: unknown } | null;
type MaskStats = {
  rect: { x: number; y: number; w: number; h: number };
  hash: string;
  sum: number;
  full: number;
  length: number;
} | null;

/** Screen point (page px) of an image point. */
async function at(page: Page, x: number, y: number) {
  const b = await stageBox(page);
  const v = await view(page);
  return { x: b.x + v.tx + x * v.scale, y: b.y + v.ty + y * v.scale };
}

async function dragImage(page: Page, from: [number, number], to: [number, number]) {
  const a = await at(page, ...from);
  const b = await at(page, ...to);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(b.x, b.y, { steps: 8 });
  await page.mouse.up();
}

const region = (page: Page) => hook<Region>(page, 'region');
const mask = (page: Page) => hook<MaskStats>(page, 'mask');

test.beforeEach(async ({ page }) => {
  await openMap(page);
  // An exact view: 2 image px per CSS px from the origin, so every point below is a whole
  // screen pixel and the image rects come out exact.
  await hook(page, 'setView', { scale: 0.5, tx: 0, ty: 0 });
});

test('FR-EDT-27 region: draw, move, resize, delete; the validated inner and margin are drawn', async ({
  page,
}) => {
  await page.keyboard.press('r');
  await dragImage(page, [400, 500], [800, 900]);
  await expect
    .poll(async () => (await region(page))?.inner)
    .toEqual({ x: 400, y: 500, w: 400, h: 400 });
  await expect(page.getByTestId('edt-region-box')).toHaveAttribute('data-rect', '400,500,400,400');
  await expect(page.getByTestId('edt-region-margin')).toBeVisible();
  await expect(page.getByTestId('edt-region-handle')).toHaveCount(8);

  // Move: drag inside.
  await dragImage(page, [600, 700], [700, 650]);
  await expect
    .poll(async () => (await region(page))?.inner)
    .toEqual({ x: 500, y: 450, w: 400, h: 400 });

  // Resize by the south-east handle.
  const se = page.locator('[data-testid="edt-region-handle"][data-handle="se"]');
  const hb = (await se.boundingBox())!;
  const target = await at(page, 1000, 1000);
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await page.mouse.move(target.x, target.y, { steps: 6 });
  await page.mouse.up();
  await expect
    .poll(async () => (await region(page))?.inner)
    .toEqual({ x: 500, y: 450, w: 500, h: 550 });

  // Too small → RGN's error, region unchanged.
  await dragImage(page, [10, 10], [14, 14]);
  await expect(page.getByTestId('edt-region-error')).toHaveAttribute(
    'data-code',
    'region_too_small',
  );
  expect((await region(page))?.inner).toEqual({ x: 500, y: 450, w: 500, h: 550 });

  await page.keyboard.press('Delete');
  await expect.poll(() => region(page)).toBeNull();
  await expect(page.getByTestId('edt-region-box')).toHaveCount(0);
  // Each change is one undo step.
  await page.keyboard.press('Control+z');
  await expect
    .poll(async () => (await region(page))?.inner)
    .toEqual({ x: 500, y: 450, w: 500, h: 550 });
});

test('FR-EDT-17 painting needs a region; the cursor shows the true and hardness radii', async ({
  page,
}) => {
  await page.keyboard.press('b');
  const p = await at(page, 1000, 1000);
  await page.mouse.move(p.x, p.y);
  await expect(page.getByTestId('edt-cursor-hint')).toHaveText('Select a region first');
  const v = await view(page);
  const r = Number(await page.getByTestId('edt-brush-cursor').getAttribute('data-radius'));
  expect(r).toBeCloseTo(32 * v.scale, 3); // default size 64 → r = 32 image px
  const hr = Number(await page.getByTestId('edt-brush-cursor-hard').getAttribute('data-radius'));
  expect(hr).toBeCloseTo(32 * v.scale * 0.5, 3);
  await expect(page.getByTestId('edt-toolbar-fill')).toBeDisabled();
});

test('FR-EDT-14 FR-EDT-15 FR-EDT-21 a stroke is one undo step; undo then redo is byte-identical', async ({
  page,
}) => {
  await page.keyboard.press('r');
  await dragImage(page, [400, 400], [1000, 1000]);
  await expect.poll(async () => (await region(page))?.inner.w).toBe(600);
  const empty = (await mask(page))!;
  expect(empty).toMatchObject({ rect: { x: 400, y: 400, w: 600, h: 600 }, sum: 0 });

  await page.keyboard.press('b');
  await dragImage(page, [500, 500], [900, 700]);
  const painted = (await mask(page))!;
  expect(painted.sum).toBeGreaterThan(0);
  // Strokes are clipped to the region: the buffer is the region, nothing else exists.
  expect(painted.length).toBe(600 * 600);

  // Eraser stroke, then undo twice and redo twice.
  await page.keyboard.press('e');
  await dragImage(page, [500, 500], [900, 700]);
  const erased = (await mask(page))!;
  expect(erased.sum).toBeLessThan(painted.sum);
  const depth = (await hook<{ depth: number }>(page, 'undoDepth')).depth;
  await page.keyboard.press('Control+z');
  expect((await mask(page))!.hash).toBe(painted.hash);
  await page.keyboard.press('Control+z');
  expect((await mask(page))!.hash).toBe(empty.hash);
  await page.keyboard.press('Control+Shift+z');
  expect((await mask(page))!.hash).toBe(painted.hash);
  await page.keyboard.press('Control+y');
  expect((await mask(page))!.hash).toBe(erased.hash);
  expect((await hook<{ depth: number }>(page, 'undoDepth')).depth).toBe(depth);
  await expect(page.getByTestId('edt-toolbar-undo').locator('..')).toHaveAttribute(
    'data-tooltip',
    new RegExp(`${depth} steps left`),
  );

  // M toggles the 50 % tint. The tint is the Image named "mask", not the Layer of that name.
  const visible = () =>
    page.evaluate(() =>
      window.__pfTest!.stage!()
        .findOne(
          (n: { getClassName: () => string; hasName: (s: string) => boolean }) =>
            n.getClassName() === 'Image' && n.hasName('mask'),
        )
        ?.isVisible(),
    );
  expect(await visible()).toBe(true);
  await page.keyboard.press('m');
  expect(await visible()).toBe(false);
});

test('FR-EDT-15 a 100 % hardness stroke is 255 inside and 0 outside', async ({ page }) => {
  await page.keyboard.press('r');
  await dragImage(page, [400, 400], [700, 700]);
  await expect.poll(async () => (await region(page))?.inner.w).toBe(300);
  await page.keyboard.press('b');
  for (let i = 0; i < 6; i++) await page.keyboard.press('Shift+]');
  await expect(page.getByTestId('edt-toolbar-hardness')).toHaveAttribute('data-value', '100');
  const p = await at(page, 550, 550);
  await page.mouse.move(p.x, p.y);
  await page.mouse.down();
  await page.mouse.up();
  const m = (await mask(page))!;
  const values = new Set(await hook<number[]>(page, 'maskBytes', 100, 100, 100, 100));
  expect([...values].sort()).toEqual([0, 255]);
  expect(m.full).toBeGreaterThan(0);
});

test('FR-EDT-16 fill region and clear mask, one undo step each', async ({ page }) => {
  await page.keyboard.press('r');
  await dragImage(page, [100, 100], [300, 200]);
  await expect.poll(async () => (await region(page))?.inner.w).toBe(200);
  await page.getByTestId('edt-toolbar-fill').click();
  expect((await mask(page))!.full).toBe(200 * 100);
  await page.getByTestId('edt-toolbar-clear').click();
  expect((await mask(page))!.sum).toBe(0);
  await page.getByTestId('edt-toolbar-undo').click();
  expect((await mask(page))!.full).toBe(200 * 100);
});

test('FR-EDT-23 a failing restore entry leaves the stack where it was and shows the code', async ({
  page,
}) => {
  await page.evaluate(() => {
    const h = (window as unknown as { __pfMockHandlers: Record<string, () => unknown> })
      .__pfMockHandlers;
    h.project_restore_revision = () => Promise.reject({ code: 'locked', message: 'locked' });
  });
  await hook(page, 'pushRevisionEntry', 'REV_PARENT', 'REV_LATER');
  const before = await hook<{ depth: number; redo: number }>(page, 'undoDepth');
  await page.keyboard.press('Control+z');
  await expect(page.getByTestId('edt-undo-error')).toHaveAttribute('data-code', 'locked');
  expect(await hook(page, 'undoDepth')).toMatchObject({ depth: before.depth, redo: before.redo });

  // When the call succeeds, undo restores the parent revision.
  await page.evaluate(() => {
    const w = window as unknown as {
      __pfMockHandlers: Record<string, (i: { rev_id: string }) => unknown>;
      __pfRestored: string[];
    };
    w.__pfRestored = [];
    w.__pfMockHandlers.project_restore_revision = (i) => {
      w.__pfRestored.push(i.rev_id);
      return { id: i.rev_id, seq: 1, created_at: '2026-11-07T11:00:00.000Z', reason: 'restore' };
    };
  });
  await page.keyboard.press('Control+z');
  await page.keyboard.press('Control+Shift+z');
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __pfRestored: string[] }).__pfRestored))
    .toEqual(['REV_PARENT', 'REV_LATER']);
});
