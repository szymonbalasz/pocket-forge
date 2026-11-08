// EDT slice 6: candidate picker + compare (FR-EDT-24, FR-EDT-25) with mocked candidates.
import { expect, test, type Page } from '@playwright/test';

import { hook, openMap } from './edt-mock';

const rect = { x: 400, y: 400, w: 600, h: 400 };
const cand = (n: number, status = 'ready', starred = false) => ({
  id: `C${n}`,
  index: n,
  status,
  starred,
  thumbUrl: status === 'ready' ? `http://pf-tiles.localhost/i/th${n}.png` : null,
  preview: status === 'ready' ? { url: `http://pf-tiles.localhost/i/pv${n}.png`, rect } : null,
});

const events = (page: Page) =>
  page.evaluate(() => (window as unknown as { __pfPickerEvents: unknown[][] }).__pfPickerEvents);
type Clip = { x: number; y: number; width: number; height: number };

test.beforeEach(async ({ page }) => {
  await openMap(page);
  await hook(page, 'setView', { scale: 0.5, tx: 0, ty: 0 });
  await hook(page, 'setCandidates', [cand(1), cand(2), cand(3, 'pending'), cand(4, 'invalid')]);
  await expect(page.getByTestId('edt-picker-item')).toHaveCount(4);
});

test('FR-EDT-25 keys and clicks select ready candidates and emit onSelect; Enter accepts once', async ({
  page,
}) => {
  const items = page.getByTestId('edt-picker-item');
  await expect(items.nth(0)).toHaveAttribute('data-record-id', 'C1');
  await expect(items.nth(2)).toHaveAttribute('data-status', 'pending');

  await page.keyboard.press('2');
  await expect(items.nth(1)).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('3'); // pending: ignored
  await page.keyboard.press('4'); // invalid: ignored
  await expect(items.nth(1)).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('ArrowRight'); // wraps over ready ones only
  await expect(items.nth(0)).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('ArrowLeft');
  await expect(items.nth(1)).toHaveAttribute('aria-selected', 'true');
  await items.nth(0).click();
  await items.nth(3).click(); // invalid: ignored
  await expect(items.nth(0)).toHaveAttribute('aria-selected', 'true');

  await page.keyboard.press('Enter');
  await page.getByTestId('edt-picker-star').nth(1).click();
  await page.getByTestId('edt-picker-discard').nth(3).click();
  await page.getByTestId('edt-picker-accept').click();
  expect(await events(page)).toEqual([
    ['select', 'C2'],
    ['select', 'C1'],
    ['select', 'C2'],
    ['select', 'C1'],
    ['accept', 'C1'],
    ['star', 'C2', true],
    ['discard', 'C4'],
    ['accept', 'C1'],
  ]);
  // Picker keys are ignored in a text field too (FR-EDT-30).
  await page.getByTestId('prj-panel-history-label').click();
  await page.keyboard.press('2');
  await expect(items.nth(0)).toHaveAttribute('aria-selected', 'true');
});

test('owner #56 X discards the selected candidate, Delete the focused one; the selection moves on', async ({
  page,
}) => {
  const items = page.getByTestId('edt-picker-item');
  await page.keyboard.press('1');
  await page.keyboard.press('x');
  // C1 discarded: the next ready candidate (C2) takes the selection.
  await expect(items.nth(1)).toHaveAttribute('aria-selected', 'true');
  await items.nth(3).focus();
  await page.keyboard.press('Delete');
  // C4 (invalid) discarded from its focused item; focus moves to the previous ready one (C2).
  await expect(items.nth(1)).toBeFocused();
  expect(await events(page)).toEqual([
    ['select', 'C1'],
    ['discard', 'C1'],
    ['select', 'C2'],
    ['discard', 'C4'],
    ['select', 'C2'],
  ]);
  // Nothing is accepted: no "Discard the rest".
  await expect(page.getByTestId('edt-picker-discard-rest')).toHaveCount(0);
});

test('owner #56 "Discard the rest" shows once a candidate is accepted; the accepted one has no Discard', async ({
  page,
}) => {
  await hook(page, 'setCandidates', [{ ...cand(1), accepted: true }, cand(2), cand(3)]);
  await expect(page.getByTestId('edt-picker-discard').nth(0)).toBeDisabled();
  await page.keyboard.press('1');
  await page.keyboard.press('x'); // the accepted one: nothing
  await page.getByTestId('edt-picker-discard-rest').click();
  expect(await events(page)).toEqual([['select', 'C1'], ['discard-rest']]);
});

test('FR-EDT-24 compare follows the selection: split divider drag, C switches to flip, hold \\ for before', async ({
  page,
}) => {
  await page.keyboard.press('1');
  await expect.poll(() => hook<boolean>(page, 'compareLoaded')).toBe(true);
  // Split, default 50 %: the after image shows right of x = 700.
  expect(await hook<Clip>(page, 'compareClip')).toEqual({
    x: 700,
    y: 400,
    width: 300,
    height: 400,
  });
  const after = (
    await hook<{ corners: { x: number; y: number }[] }[]>(page, 'find', '.compare-after')
  )[0]!;
  expect(after.corners[0]).toEqual({ x: 400, y: 400 });
  expect(after.corners[2]).toEqual({ x: 1000, y: 800 });

  // Drag the divider to 25 % (image x 550 = screen 275 at scale 0.5).
  const div = page.getByTestId('edt-compare-divider');
  await expect(div).toHaveAttribute('data-split', '0.5000');
  const b = (await div.boundingBox())!;
  const stage = (await page.getByTestId('edt-canvas-stage').boundingBox())!;
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
  await page.mouse.down();
  await page.mouse.move(stage.x + 275, b.y + b.height / 2, { steps: 5 });
  await page.mouse.up();
  await expect(div).toHaveAttribute('data-split', '0.2500');
  expect((await hook<Clip>(page, 'compareClip')).x).toBe(550);

  // Zoom: the after image stays on its rect in image px.
  await page.keyboard.press('Control+1');
  expect((await hook<Clip>(page, 'compareClip')).x).toBe(550);

  // C → flip: all of the after image; hold \ → before.
  await page.keyboard.press('c');
  await expect(page.getByTestId('edt-compare-mode')).toHaveAttribute('data-mode', 'flip');
  await expect(div).toHaveCount(0);
  expect(await hook<Clip>(page, 'compareClip')).toEqual({
    x: 400,
    y: 400,
    width: 600,
    height: 400,
  });
  await page.keyboard.down('Backslash');
  await expect.poll(async () => (await hook<Clip>(page, 'compareClip')).width).toBe(0);
  await page.keyboard.up('Backslash');
  await expect.poll(async () => (await hook<Clip>(page, 'compareClip')).width).toBe(600);

  // Selecting another candidate switches the compared image (its preview URL is fetched).
  const fetched: string[] = [];
  page.on('request', (r) => fetched.push(r.url()));
  await page.keyboard.press('2');
  await expect.poll(() => fetched).toContain('http://pf-tiles.localhost/i/pv2.png');
  await expect.poll(() => hook<boolean>(page, 'compareLoaded')).toBe(true);
});

test('FR-EDT-25 Enter and Space on a focused button press it: Fit runs, no accept (#48)', async ({
  page,
}) => {
  await page.keyboard.press('1');
  await expect(page.getByTestId('edt-picker-item').nth(0)).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('Control+0');
  await page.keyboard.press('r'); // not the pan tool, so the pan cursor only means Space is held
  const fitted = await hook<{ scale: number }>(page, 'view');
  const away = { scale: fitted.scale * 3, tx: -100, ty: -50 };
  const fit = page.getByTestId('edt-view-fit');

  // Enter on the focused Fit button runs Fit and doesn't accept the selected candidate.
  await hook(page, 'setView', away);
  await fit.focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => hook(page, 'view')).toEqual(fitted);

  // Space presses the button too (it doesn't hold temporary pan).
  await hook(page, 'setView', away);
  await fit.focus();
  await page.keyboard.press(' ');
  await expect.poll(() => hook(page, 'view')).toEqual(fitted);
  await expect(page.getByTestId('edt-canvas-stage')).not.toHaveClass(/editor-canvas-pan/);

  expect(await events(page)).toEqual([['select', 'C1']]);
});
