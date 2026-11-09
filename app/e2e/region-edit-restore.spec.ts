// RGN walkthrough D1/D2 (walk-region-edit): a restore keeps EDT's live region and mask
// (OQ-EDT-14 A, FR-EDT-23) and pushes no editor undo entry, so Redo works (FR-RGN-30); a region
// drawn after Delete or a restore reaches the Generate panel. Mocked core (KQ-33).
import { expect, test, type Page } from '@playwright/test';

import { hook } from './edt-mock';
import { drawRegion, drive, inputsOf, openRgn } from './rgn-mock';

type Rect = { x: number; y: number; w: number; h: number };
type Undo = { depth: number; redo: number; label: string | null };

const region = async (page: Page) =>
  (await hook<{ inner: Rect } | null>(page, 'region'))?.inner ?? null;
const maskFull = async (page: Page) => (await hook<{ full: number } | null>(page, 'mask'))?.full;
const undo = (page: Page) => hook<Undo>(page, 'undoDepth');

/** A region with the whole mask filled, saved to the store (FR-EDT-28). */
async function regionAndFill(page: Page, from: [number, number], to: [number, number]) {
  await drawRegion(page, from, to);
  await expect.poll(() => region(page)).not.toBeNull();
  await page.getByTestId('edt-toolbar-mask-region').click();
  await expect(page.getByTestId('edt-mask-status')).toHaveText('Mask saved', { timeout: 6000 });
}

/** Region, mask, prompt → submit → 3 candidates → accept #1 (r1 `accept`). */
async function acceptOne(page: Page) {
  await regionAndFill(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('CU');
  await page.getByTestId('rgn-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await drive(page, 'finish', 3);
  await expect(page.getByTestId('edt-picker-item')).toHaveCount(3);
  await page.keyboard.press('1');
  await page.getByTestId('edt-picker-accept').click();
  await expect(page.getByTestId('rgn-tracker-accepted')).toBeVisible();
}

test('FR-RGN-30 OQ-EDT-14 undo of an accept keeps the live region and mask; redo restores the later revision (walk D1)', async ({
  page,
}) => {
  await openRgn(page, { maskId: null });
  await acceptOne(page);
  const inner = await region(page);
  const full = await maskFull(page);
  expect(full).toBe(400 * 400);
  await expect.poll(async () => (await undo(page)).label).toBe('Accept patch');
  const { depth } = await undo(page);

  await page.keyboard.press('Control+z');
  await expect.poll(async () => (await inputsOf(page, 'project_restore_revision')).length).toBe(1);
  // r0's draft is null: EDT's live region and mask stay, and no editor entry is pushed.
  await expect.poll(async () => (await undo(page)).redo).toBe(1);
  expect(await undo(page)).toMatchObject({ depth: depth - 1, redo: 1 });
  expect(await region(page)).toEqual(inner);
  expect(await maskFull(page)).toBe(full);
  await expect(page.getByTestId('rgn-panel-size')).toHaveText('400 × 400 px');

  await page.keyboard.press('Control+Shift+z');
  await expect.poll(async () => (await inputsOf(page, 'project_restore_revision')).length).toBe(2);
  await expect.poll(async () => (await undo(page)).depth).toBe(depth);
  expect(await undo(page)).toMatchObject({ redo: 0, label: 'Accept patch' });
  expect(await region(page)).toEqual(inner);
  expect(await maskFull(page)).toBe(full);
});

test('FR-RGN-30 OQ-EDT-14 after a restore the draft points at the live region and mask', async ({
  page,
}) => {
  await openRgn(page, { maskId: null });
  await acceptOne(page);
  const masks = (await inputsOf(page, 'edt_mask_submit')).length;
  await page.keyboard.press('Control+z');
  await expect.poll(async () => (await undo(page)).redo).toBe(1);
  // The restored draft (r0: none) is replaced by the live region and the stored live mask, so a
  // reopen shows what the DM sees; the mask bytes are unchanged, so no second mask file.
  await expect
    .poll(async () => (await inputsOf(page, 'project_put_draft')).at(-1)?.draft, {
      timeout: 5000,
    })
    .toMatchObject({
      region: { inner: { x: 400, y: 500, w: 400, h: 400 } },
      mask_id: expect.stringMatching(/^01JMASK/),
    });
  expect((await inputsOf(page, 'edt_mask_submit')).length).toBe(masks);
});

test('FR-RGN-01 a region drawn after Delete reaches the Generate panel (walk D2)', async ({
  page,
}) => {
  await openRgn(page, { maskId: null });
  await regionAndFill(page, [400, 500], [800, 900]);
  await expect(page.getByTestId('rgn-panel-mask-hint')).toBeHidden();
  await page.keyboard.press('Delete');
  await expect(page.getByTestId('rgn-panel-empty')).toBeVisible();
  await regionAndFill(page, [1000, 500], [1300, 800]);
  await expect(page.getByTestId('rgn-panel-size')).toHaveText('300 × 300 px');
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-submit')).toBeEnabled();
});

test('FR-RGN-01 a region drawn after a restore reaches the Generate panel (walk D2)', async ({
  page,
}) => {
  await openRgn(page, { maskId: null });
  await acceptOne(page);
  await page.keyboard.press('Control+z');
  await expect.poll(async () => (await inputsOf(page, 'project_restore_revision')).length).toBe(1);
  await expect(page.getByTestId('prm-block-prompt')).toHaveValue('');
  await page.keyboard.press('Delete');
  await expect(page.getByTestId('rgn-panel-empty')).toBeVisible();
  await regionAndFill(page, [1000, 500], [1300, 800]);
  await expect(page.getByTestId('rgn-panel-size')).toHaveText('300 × 300 px');
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-submit')).toBeEnabled();
});
