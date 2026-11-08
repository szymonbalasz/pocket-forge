// EDT slice 5: mask hand-off (FR-EDT-18, 28, 29), reopen through the real submit (FR-EDT-19),
// the stable original id (#48 (3)) and a missing original (BL-10). Mocked core (KQ-33).
import { expect, test, type Page } from '@playwright/test';

import { hook, installMap, openMap, stageBox, view } from './edt-mock';
import { calls, openFirstCard } from './mock-core';
import { drawRegion, openRgn } from './rgn-mock';

type Rect = { x: number; y: number; w: number; h: number };
type Submit = { cmd: string; args: { input: { rect: Rect; data_b64: string } } };

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
  await page.mouse.move(b.x, b.y, { steps: 6 });
  await page.mouse.up();
}

/** A 200×100 region at (400, 500) with a brush stroke across it. */
async function regionAndStroke(page: Page) {
  await hook(page, 'setView', { scale: 0.5, tx: 0, ty: 0 });
  await page.keyboard.press('r');
  await dragImage(page, [400, 500], [600, 600]);
  await expect
    .poll(async () => (await hook<{ inner: Rect } | null>(page, 'region'))?.inner)
    .toEqual({ x: 400, y: 500, w: 200, h: 100 });
  await page.keyboard.press('b');
  await dragImage(page, [420, 550], [580, 550]);
}

const submits = async (page: Page) =>
  (await calls(page)).filter((c) => c.cmd === 'edt_mask_submit') as unknown as Submit[];

const status = (page: Page) => page.getByTestId('edt-mask-status');

/** The live mask's bytes (the whole 200×100 region). */
const liveBytes = (page: Page) => hook<number[]>(page, 'maskBytes', 0, 0, 200, 100);

const decode = (b64: string) => Array.from(Buffer.from(b64, 'base64'));

test('FR-EDT-28 FR-EDT-18 a stroke is submitted once after 2 s idle, byte-identical', async ({
  page,
}) => {
  await openMap(page);
  await regionAndStroke(page);
  await expect(status(page)).toHaveAttribute('data-state', 'unsaved:pending');
  await page.waitForTimeout(1200);
  expect(await submits(page)).toHaveLength(0); // still inside the 2 s debounce
  await expect(status(page)).toHaveAttribute('data-state', 'saved', { timeout: 5000 });
  await expect(status(page)).toHaveText('Mask saved');
  const sent = await submits(page);
  expect(sent).toHaveLength(1);
  expect(sent[0]!.args.input.rect).toEqual({ x: 400, y: 500, w: 200, h: 100 });
  const bytes = decode(sent[0]!.args.input.data_b64);
  expect(bytes).toEqual(await liveBytes(page));
  expect(bytes.some((v) => v === 255)).toBe(true);
  // Nothing changed since: no second submit.
  await page.waitForTimeout(2500);
  expect(await submits(page)).toHaveLength(1);
});

test('FR-EDT-19 FR-EDT-28 close submits the pending mask; reopen restores it pixel-identically', async ({
  page,
}) => {
  await openMap(page);
  await regionAndStroke(page);
  const before = await liveBytes(page);
  // Close right away, inside the debounce: the mask is submitted before project_close.
  await page.getByTestId('prj-panel-close').click();
  await expect(page.getByTestId('prj-home-card').first()).toBeVisible();
  const names = (await calls(page)).map((c) => c.cmd);
  const submitAt = names.lastIndexOf('edt_mask_submit');
  expect(submitAt).toBeGreaterThan(-1);
  expect(submitAt).toBeLessThan(names.lastIndexOf('project_close'));

  await openFirstCard(page);
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  await expect
    .poll(async () => (await hook<{ inner: Rect } | null>(page, 'region'))?.inner)
    .toEqual({ x: 400, y: 500, w: 200, h: 100 });
  expect(await liveBytes(page)).toEqual(before);
  await expect(status(page)).toHaveAttribute('data-state', 'saved');
  // The reopened mask isn't submitted again.
  await page.waitForTimeout(2500);
  expect(await submits(page)).toHaveLength(1);
});

test('FR-EDT-28 R7 the job-submit trigger flushes at once and returns the new ref', async ({
  page,
}) => {
  await openMap(page);
  await regionAndStroke(page);
  const ref = await hook<{ mask_id: string; rect: Rect }>(page, 'flushMask');
  expect(ref.rect).toEqual({ x: 400, y: 500, w: 200, h: 100 });
  expect(ref.mask_id).toMatch(/^01JMASK/);
  expect(await submits(page)).toHaveLength(1);
  await expect(status(page)).toHaveAttribute('data-state', 'saved');
});

test('FR-EDT-29 read-only: painting works, no submit is attempted, the status says so', async ({
  page,
}) => {
  await openMap(page, { readOnly: 'newer_version' });
  await regionAndStroke(page);
  expect((await liveBytes(page)).some((v) => v > 0)).toBe(true);
  await expect(status(page)).toHaveText('Read-only: mask not saved');
  await page.keyboard.press('Control+z');
  await expect.poll(async () => (await liveBytes(page)).every((v) => v === 0)).toBe(true);
  await page.waitForTimeout(2500);
  expect(await hook(page, 'flushMask')).toBeNull();
  expect(await submits(page)).toHaveLength(0);
});

test('EDT §11 a locked write shows unsaved and keeps the mask', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, Record<string, unknown>>;
    (w.__pfMockHandlers ??= {}).edt_mask_submit = () =>
      Promise.reject({ code: 'locked', message: 'the file is locked' });
  });
  await openMap(page);
  await regionAndStroke(page);
  await expect(status(page)).toHaveAttribute('data-state', 'unsaved:locked', { timeout: 5000 });
  await expect(status(page)).toContainText('locked');
  expect((await liveBytes(page)).some((v) => v === 255)).toBe(true);
});

test('#48 (3) tiles open with the stable original image id', async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as Record<string, unknown>).__pfMockSnapshot = {
      original_image_id: '01JORIGINAL0000000000000000',
    };
  });
  await openMap(page);
  const opened = (await page.evaluate(
    () => (window as unknown as { __pfEdtTilesOpen: { image_id: string }[] }).__pfEdtTilesOpen,
  ))!;
  // StrictMode runs effects twice in dev; `edt_tiles_open` is idempotent. Every call uses it.
  expect(opened.length).toBeGreaterThan(0);
  expect(new Set(opened.map((c) => c.image_id))).toEqual(new Set(['01JORIGINAL0000000000000000']));
});

test('BL-10 a missing original: a readable message, no Rebuild, no tiles, tools disabled', async ({
  page,
}) => {
  await installMap(page, { readOnly: 'original_missing' });
  await page.goto('/');
  await openFirstCard(page);
  await expect(page.getByTestId('edt-original-missing')).toContainText('Relink');
  await expect(page.getByTestId('edt-build-error')).toHaveCount(0);
  await expect(page.getByText('Rebuild')).toHaveCount(0);
  await expect(page.getByTestId('edt-loading')).toHaveCount(0);
  for (const t of ['pan', 'region', 'brush', 'eraser', 'fill', 'clear'])
    await expect(page.getByTestId(`edt-toolbar-${t}`)).toBeDisabled();
  expect((await calls(page)).some((c) => c.cmd === 'edt_tiles_open')).toBe(false);
  await page.keyboard.press('b');
  expect((await hook<{ tool: string }>(page, 'store')).tool).toBe('pan');
});

test('BL-14 FR-EDT-27 a margin change in Generate redraws the outer rect; the mask is untouched', async ({
  page,
}) => {
  await openRgn(page);
  await drawRegion(page, [400, 500], [800, 900]);
  type Region = { inner: Rect; outer: Rect } | null;
  await expect
    .poll(async () => (await hook<Region>(page, 'region'))?.outer)
    .toEqual({ x: 272, y: 372, w: 656, h: 656 });
  await page.getByTestId('edt-toolbar-fill').click();
  const depth = (await hook<{ depth: number }>(page, 'undoDepth')).depth;
  const margin = page.getByTestId('rgn-panel-margin');
  await margin.fill('64');
  await margin.press('Enter');
  await expect
    .poll(async () => (await hook<Region>(page, 'region'))?.outer)
    .toEqual({ x: 336, y: 436, w: 528, h: 528 });
  expect((await hook<Region>(page, 'region'))?.inner).toEqual({ x: 400, y: 500, w: 400, h: 400 });
  // The drawn outline follows; no undo step, the mask is still full.
  const box = await page.getByTestId('edt-region-margin').boundingBox();
  expect(Math.round(box!.width)).toBe(264); // 528 image px at scale 0.5
  expect((await hook<{ depth: number }>(page, 'undoDepth')).depth).toBe(depth);
  expect((await hook<{ full: number }>(page, 'mask')).full).toBe(400 * 400);
});
