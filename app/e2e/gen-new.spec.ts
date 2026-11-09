// GEN thin e2e on the mocked core (new-asset-generation §13; slice 5, UI on mocked commands):
// + New menu → Generate new… → name → the project opens on the New image tab → size, prompt →
// estimate → confirm → the job runs in RGN's tracker with the `new image` badge. Slice 4: pick
// (alone preview with no image yet), undo back to "No image yet", re-pick from an older iteration.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { expect, test } from '@playwright/test';

import type { Capabilities } from '../src/api';
import { installGenMock } from '../src/api/mocks/gen';
import { installRgnMock } from '../src/api/mocks/rgn';
import { drawRegion, drive, inputsOf, rgnSeed } from './rgn-mock';
import { hook, installMap } from './edt-mock';

const GEN = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../fixtures/gen/capabilities-generate.json', import.meta.url)),
    'utf8',
  ),
) as Record<string, Capabilities>;

test.beforeEach(async ({ page }) => {
  // Init scripts run in order: the feature mocks first, so mock-core keeps their handlers.
  await page.addInitScript(installRgnMock, rgnSeed());
  await page.addInitScript(installGenMock, {
    models: [
      { id: 'fake-gen-wh', name: 'Fake gen (wh)' },
      { id: 'fake-gen-aspect', name: 'Fake gen (aspect)' },
    ],
    capabilities: GEN,
    costPerImage: 1,
  });
  // The map mocks (tiles, mask store, test hooks) on mock-core: region edit after a pick.
  await installMap(page);
  await page.goto('/');
});

test('FR-GEN-01 FR-GEN-04 FR-GEN-08 FR-GEN-21 FR-GEN-25 + New → Generate new… → generate on the New image tab', async ({
  page,
}) => {
  await page.getByTestId('prj-home-new').click();
  await page.getByTestId('prj-home-new-generate').click();
  const name = page.getByTestId('gen-new-name');
  await expect(name).toHaveValue('New map');
  await expect(name).toBeFocused();
  await name.fill('Crypt');
  await page.getByTestId('gen-new-ok').click();

  // FR-GEN-03/04: no image yet → only the New image tab.
  await expect(page.getByTestId('gen-panel')).toBeVisible();
  await expect(page.getByTestId('gen-tab')).toHaveCount(1);
  await expect(page.getByTestId('gen-tab')).toHaveAttribute('data-record-id', 'new-image');
  expect((await inputsOf(page, 'gen_project_create')).at(-1)).toEqual({
    section: 'maps',
    name: 'Crypt',
  });

  // Walkthrough step 2: fake-gen-wh, 30 × 40 at 70 → model limit.
  await expect(page.getByTestId('gen-size-result')).toHaveText(
    'sent 1520 × 2032 · 50.7 × 50.8 px per square',
  );
  await expect(page.getByTestId('gen-size-notices')).toContainText('Model limit');

  await page.getByTestId('prm-block-prompt').fill('flooded crypt, torchlight');
  await expect(page.getByTestId('gen-panel-estimate')).toHaveAttribute('data-status', 'ready');
  await page.getByTestId('gen-panel-submit').click();
  await expect(page.getByTestId('gen-confirm-summary')).toHaveText('New image · 1520 × 2032');
  await page.getByTestId('rgn-confirm-ok').click();
  await expect(page.getByTestId('rgn-confirm')).toBeHidden();

  await expect(page.getByTestId('gen-panel-submit')).toBeDisabled();
  await drive(page, 'finish');
  await expect(page.getByTestId('gen-tracker-badge')).toHaveText('new image · 1520 × 2032');
  await expect(page.getByTestId('rgn-tracker-item')).toHaveAttribute('data-n', '1');
  const sent = (await inputsOf(page, 'gen_job_submit')).at(-1)!;
  expect(sent.draft).toMatchObject({
    model_id: 'fake-gen-wh',
    typed_prompt: 'flooded crypt, torchlight',
    size: { mode: 'grid', cols: 30, rows: 40, pps: 70 },
    structure: null,
  });
});

async function newProject(page: import('@playwright/test').Page, name: string) {
  await page.getByTestId('prj-home-new').click();
  await page.getByTestId('prj-home-new-generate').click();
  await page.getByTestId('gen-new-name').fill(name);
  await page.getByTestId('gen-new-ok').click();
  await expect(page.getByTestId('gen-panel')).toBeVisible();
}

async function generate(page: import('@playwright/test').Page, prompt: string) {
  await page.getByTestId('prm-block-prompt').fill(prompt);
  await expect(page.getByTestId('gen-panel-estimate')).toHaveAttribute('data-status', 'ready');
  await expect(page.getByTestId('gen-panel-submit')).toBeEnabled();
  await page.getByTestId('gen-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await expect(page.getByTestId('rgn-confirm')).toBeHidden();
  await drive(page, 'finish', 2);
  await expect(page.getByTestId('edt-picker-item')).toHaveCount(2);
}

test('FR-GEN-26 FR-GEN-27 FR-GEN-29 FR-GEN-30 generate → pick → undo → re-pick from an older iteration', async ({
  page,
}) => {
  await newProject(page, 'Crypt');
  await expect(page.getByTestId('edt-canvas-no-image')).toBeVisible();
  await generate(page, 'flooded crypt');

  // FR-GEN-26: no image yet → the candidate is shown alone, fit to view.
  await page.getByTestId('edt-picker-item').nth(1).click();
  await expect(page.getByTestId('edt-preview-alone')).toBeVisible();
  expect(await inputsOf(page, 'rgn_candidate_preview')).toHaveLength(0);
  // Walk L7: no split / flip toggle while the candidate is shown alone.
  await expect(page.getByTestId('edt-compare-mode')).toHaveCount(0);

  // FR-GEN-27: Pick (Enter) makes it the bottom image; the canvas replaces "No image yet".
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('edt-canvas-stage')).toBeVisible();
  await expect(page.getByTestId('edt-canvas-no-image')).toHaveCount(0);
  const first = (await inputsOf(page, 'gen_candidate_pick')).at(-1)!;
  expect(await inputsOf(page, 'rgn_candidate_accept')).toHaveLength(0);
  await expect(page.getByTestId('rgn-tracker-item')).toHaveAttribute('data-accepted', 'true');

  // FR-GEN-29: undo the first pick → "No image yet" again.
  await page.getByTestId('edt-toolbar-undo').click();
  await expect(page.getByTestId('edt-canvas-no-image')).toBeVisible();
  expect((await inputsOf(page, 'project_restore_revision')).at(-1)).toEqual({ rev_id: 'r0' });

  // A second iteration, then back to the older 001 and pick its other candidate (OQ-GEN-08).
  await generate(page, 'dry crypt');
  await page.locator('[data-testid="rgn-tracker-item"][data-n="1"] .rgn-tracker-n').click();
  await expect(page.getByTestId('edt-picker-item')).toHaveCount(2);
  await page.getByTestId('edt-picker-item').nth(0).click();
  await expect(page.getByTestId('edt-preview-alone')).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('edt-canvas-stage')).toBeVisible();
  const repick = (await inputsOf(page, 'gen_candidate_pick')).at(-1)!;
  expect(repick.job_id).toBe(first.job_id);
  expect(repick.candidate_id).not.toBe(first.candidate_id);
  expect(await inputsOf(page, 'gen_candidate_pick')).toHaveLength(2);
});

test('walk L1 FR-GEN-29 FR-EDT-23 keyboard undo and redo across pick → patch → re-pick', async ({
  page,
}) => {
  type Undo = { depth: number; redo: number; label: string | null };
  const undo = () => hook<Undo>(page, 'undoDepth');
  const restores = async () => (await inputsOf(page, 'project_restore_revision')).length;
  await newProject(page, 'Crypt');
  await generate(page, 'flooded crypt');
  await page.getByTestId('edt-picker-item').nth(0).click();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('edt-canvas-stage')).toBeVisible();

  // Region edit on the picked image: region, whole mask, generate, accept #1.
  await page.locator('[data-testid="gen-tab"][data-record-id="region"]').click();
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('edt-toolbar-mask-region').click();
  await expect(page.getByTestId('edt-mask-status')).toHaveText('Mask saved', { timeout: 6000 });
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('CU');
  await page.getByTestId('rgn-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await drive(page, 'finish', 3);
  await expect(page.getByTestId('edt-picker-item')).toHaveCount(3);
  await page.keyboard.press('1');
  await page.getByTestId('edt-picker-accept').click();
  await expect(page.getByTestId('rgn-tracker-accepted')).toHaveCount(2);

  // Re-pick #2 of 001.
  await page.locator('[data-testid="rgn-tracker-item"][data-n="1"] .rgn-tracker-n').click();
  await expect(page.getByTestId('edt-picker-item')).toHaveCount(2);
  const previews = (await inputsOf(page, 'gen_candidate_preview')).length;
  await page.getByTestId('edt-picker-item').nth(1).click();
  await expect
    .poll(async () => (await inputsOf(page, 'gen_candidate_preview')).length)
    .toBe(previews + 1);
  await page.keyboard.press('Enter');
  await expect.poll(async () => (await undo()).label).toBe('Pick new image');
  const top = await undo();

  // Ctrl+Z twice: the re-pick, then the patch.
  await page.keyboard.press('Control+z');
  await expect.poll(restores).toBe(1);
  // No editor entry is pushed by the remount, so Redo stays (walk L1).
  await expect.poll(async () => (await undo()).redo).toBe(1);
  expect((await undo()).label).toBe('Accept patch');
  await page.keyboard.press('Control+z');
  await expect.poll(restores).toBe(2);
  await expect.poll(async () => (await undo()).label).toBe('Pick new image');
  expect((await undo()).redo).toBe(2);
  // Redo both back, in order.
  await page.keyboard.press('Control+Shift+z');
  await expect.poll(restores).toBe(3);
  await expect.poll(async () => (await undo()).redo).toBe(1);
  await page.keyboard.press('Control+Shift+z');
  await expect.poll(restores).toBe(4);
  await expect.poll(async () => (await undo()).redo).toBe(0);
  expect((await undo()).depth).toBe(top.depth);
});
