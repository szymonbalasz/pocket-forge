// Owner decisions after the first real run (2026-11-08) and #55, on the mocked core (KQ-33):
// the empty-mask hint and Mask whole region, one result per job for a model without a count
// control, and the blend instruction (toggle, final prompt, Settings text).
import { expect, test } from '@playwright/test';

import { hook } from './edt-mock';
import { ALL_TYPES, drawRegion, inputsOf, openRgn } from './rgn-mock';

test('FR-RGN-03 owner-2026-11-08 no painted mask: Generate disabled with a hint; Mask whole region fixes it in one undo step', async ({
  page,
}) => {
  await openRgn(page, { maskId: null });
  const maskRegion = page.getByTestId('edt-toolbar-mask-region');
  await expect(maskRegion).toBeDisabled(); // no region yet
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('rgn-panel-prompt').fill('A stone well');
  const hint = page.getByTestId('rgn-panel-mask-hint');
  await expect(hint).toHaveText('Paint a mask inside the region to choose what changes.');
  await expect(page.getByTestId('rgn-panel-submit')).toBeDisabled();

  await expect(maskRegion).toBeEnabled();
  await maskRegion.click();
  await expect(hint).toBeHidden();
  expect((await hook<{ full: number }>(page, 'mask')).full).toBe(400 * 400);
  expect((await hook<{ label: string | null }>(page, 'undoDepth')).label).toBe('mask whole region');
  // The mask goes through the existing hand-off (edt_mask_submit), then Generate is possible.
  await expect.poll(async () => (await inputsOf(page, 'edt_mask_submit')).length).toBe(1);
  await page.getByTestId('rgn-panel-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('CU');
  await expect(page.getByTestId('rgn-panel-submit')).toBeEnabled();

  // Clear mask empties it again: the hint is back.
  await page.getByTestId('edt-toolbar-clear').click();
  await expect(hint).toBeVisible();
  await expect(page.getByTestId('rgn-panel-submit')).toBeDisabled();
  // Undo restores the painted mask.
  await page.keyboard.press('Control+z');
  await expect(hint).toBeHidden();
});

test('FR-RGN-08 owner-2026-11-08 a model without a count control shows 1 result per job', async ({
  page,
}) => {
  const one = {
    ...ALL_TYPES,
    model_id: 'fake-one',
    controls: ALL_TYPES.controls.filter((c) => c.role !== 'candidate_count'),
  };
  await openRgn(page, {
    models: [{ id: 'fake-one', name: 'One result' }],
    capabilities: { 'fake-one': one },
  });
  await expect(page.getByTestId('rgn-panel-count-single')).toHaveText(
    '1 result per job (this model)',
  );
});

test('owner-55 blend instruction: on by default, in the final prompt once, Settings text used', async ({
  page,
}) => {
  await openRgn(page);
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('edt-toolbar-mask-region').click();
  await page.getByTestId('rgn-panel-prompt').fill('A stone well');
  const toggle = page.getByTestId('rgn-panel-blend-instruction');
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await page.getByTestId('rgn-panel-final').locator('summary').click();
  await expect(page.getByTestId('rgn-panel-final')).toContainText('Final prompt (as sent)');
  const shown = page.getByTestId('rgn-panel-final-prompt');
  await expect(shown).toContainText('A stone well');
  await expect(shown).toContainText('Keep everything near the edges');
  await expect(shown).toContainText('Top-down battlemap view.');

  // Off: the typed prompt only; saved in the draft.
  await toggle.click();
  await expect(shown).toHaveText('A stone well');
  await expect
    .poll(async () => (await inputsOf(page, 'project_put_draft')).at(-1)?.draft, {
      timeout: 5000,
    })
    .toMatchObject({ blend_instruction: false });
  await toggle.click();

  // Settings: edit the text; the preview follows.
  await page.getByTestId('prj-rail-settings').click();
  const text = page.getByTestId('rgn-settings-blend-text');
  await expect(text).toHaveValue(/^Keep everything near the edges/);
  await text.fill('Match the ink.');
  await page.getByTestId('rgn-settings-blend-save').click();
  await expect(page.getByTestId('rgn-settings-blend-result')).toHaveText('Saved.');
  await page.keyboard.press('Escape');
  await expect(shown).toHaveText('A stone well\n\nMatch the ink. Top-down battlemap view.');
});
