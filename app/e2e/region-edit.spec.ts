// RGN slice 6 thin e2e (RGN §13, KQ-33): the Generate panel, confirm dialog, job strip and
// iteration tracker on the mocked core (`src/api/mocks/rgn.ts`). No provider, no spend.
import { expect, test } from '@playwright/test';

import { choose, drawMaskedRegion, drive, inputsOf, openRgn } from './rgn-mock';

test('FR-RGN-27 region → controls → prompt → estimate → confirm → progress → pick → accept', async ({
  page,
}) => {
  await openRgn(page);
  await expect(page.getByTestId('rgn-panel-empty')).toBeVisible();
  // ui-guidelines: the right panel holds Generate, then Export.
  const order = await page
    .getByTestId('app-slot-right')
    .locator('[data-testid="rgn-panel"], [data-testid="exp-panel"]')
    .evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')));
  expect(order).toEqual(['rgn-panel', 'exp-panel']);

  // Region: EDT's tool emits {inner}; RGN fits it (FR-RGN-01/05).
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await expect(page.getByTestId('rgn-panel-size')).toHaveText('400 × 400 px');
  await expect(page.getByTestId('rgn-panel-fit')).toHaveText('sent unresampled');

  // Controls from the capabilities (FR-RGN-08) and their rules (FR-RGN-33).
  const controls = page.getByTestId('rgn-control-input');
  await expect(controls).toHaveCount(7);
  await expect(page.getByTestId('rgn-panel-warning')).toContainText('"lora"');
  await choose(page, 'mode', 'quality');
  await expect(controls).toHaveCount(8); // strength shows
  await expect(
    page.locator('[data-testid="rgn-control-input"][data-record-id="style"]'),
  ).toHaveAttribute('data-required', 'true');
  await page.getByTestId('prm-block-prompt').fill('Turn this room into a kitchen');
  await expect(page.getByTestId('rgn-panel-submit-reason')).toHaveText('Required: style.');
  await choose(page, 'style', 'ink');

  // Estimate (FR-RGN-12), then confirm (FR-RGN-14).
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('Estimate: 2 CU');
  await page.getByTestId('rgn-panel-submit').click();
  const dlg = page.getByTestId('rgn-confirm');
  await expect(dlg).toBeVisible();
  await expect(dlg.getByTestId('rgn-confirm-spend')).toHaveText('2 CU');
  await page.keyboard.press('Enter'); // within the threshold: Enter confirms
  await expect(dlg).toBeHidden();
  const submit = (await inputsOf(page, 'rgn_job_submit'))[0]!;
  expect(submit.draft).toMatchObject({
    final_prompt: 'Turn this room into a kitchen',
    model_id: 'fake-all-types',
    controls: { mode: 'quality', style: 'ink' },
    region: { inner: { x: 400, y: 500, w: 400, h: 400 }, margin: 128 },
  });

  // Progress in the job strip and the status bar (FR-RGN-18).
  const item = page.getByTestId('rgn-tracker-item');
  await expect(item).toHaveCount(1);
  await expect(page.getByTestId('rgn-job-state')).toHaveText('Submitted');
  await drive(page, 'run', 0.5);
  await expect(page.getByTestId('rgn-job-progress')).toHaveText('50 %');
  await expect(page.getByTestId('rgn-status-job')).toHaveText('Job: Running 50 %');
  await expect(page.getByTestId('rgn-panel-submit')).toBeDisabled();

  // Candidates in EDT's picker (FR-RGN-40, R19); pick → preview → accept.
  await drive(page, 'finish', 3);
  await expect(item).toHaveAttribute('data-n', '1');
  await expect(item).toContainText('001');
  await expect(item).toContainText('turn this room into a kitchen');
  const picks = page.getByTestId('edt-picker-item');
  await expect(picks).toHaveCount(3);
  await page.keyboard.press('2');
  await expect(picks.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect.poll(async () => (await inputsOf(page, 'rgn_candidate_preview')).length).toBe(1);
  await expect(page.getByTestId('rgn-panel-blend')).toBeVisible();
  await page.getByTestId('rgn-blend-feather').fill('96');
  await expect.poll(async () => (await inputsOf(page, 'rgn_candidate_preview')).length).toBe(2);
  await page.getByTestId('edt-picker-accept').click();
  await expect(item).toHaveAttribute('data-accepted', 'true');
  await expect(page.getByTestId('rgn-tracker-accepted')).toBeVisible();
  const accept = (await inputsOf(page, 'rgn_candidate_accept'))[0]!;
  expect(accept.blend).toEqual({ feather_px: 96, colour_match: true });
  await expect(page.getByTestId('rgn-status-job')).toBeHidden();
});

test('FR-RGN-21 R62 Retry is offered on the newest job once it is closed by an accept (walk D4)', async ({
  page,
}) => {
  await openRgn(page);
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');
  await page.getByTestId('rgn-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await drive(page, 'finish', 3);
  await expect(page.getByTestId('edt-picker-item')).toHaveCount(3);
  await page.keyboard.press('1');
  await page.getByTestId('edt-picker-accept').click();
  const item = page.getByTestId('rgn-tracker-item');
  await expect(item).toHaveAttribute('data-state', 'closed');
  await item.getByTestId('rgn-job-retry').click();
  await expect.poll(async () => (await inputsOf(page, 'rgn_job_retry_draft')).length).toBe(1);
  await expect(page.getByTestId('prm-block-prompt')).toHaveValue('A stone well');
});

test('FR-RGN-28 a job closed by the next submit shows Closed on its chip (walk D6)', async ({
  page,
}) => {
  await openRgn(page);
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');
  await page.getByTestId('rgn-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await drive(page, 'finish', 3);
  const first = page.getByTestId('rgn-tracker-item');
  await expect(first).toHaveAttribute('data-state', 'fetched');
  await page.getByTestId('rgn-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await expect(page.getByTestId('rgn-tracker-item')).toHaveCount(2);
  const old = page.getByTestId('rgn-tracker-item').nth(1);
  await expect(old).toHaveAttribute('data-state', 'closed');
  await expect(old).toContainText('Closed');
});

test('FR-RGN-24 a candidate in another aspect shows the warning badge in the picker (walk D3)', async ({
  page,
}) => {
  await openRgn(page);
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');
  await page.getByTestId('rgn-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await drive(page, 'finish', 3, [], [1]);
  const picks = page.getByTestId('edt-picker-item');
  await expect(picks).toHaveCount(3);
  await expect(page.getByTestId('edt-picker-warning')).toHaveCount(1);
  const badge = picks.nth(1).getByTestId('edt-picker-warning');
  await expect(badge).toHaveAttribute('title', /Aspect differs/);
});

test('FR-RGN-14 over-cap dialog: spend, estimate and cap; an explicit raise, then submit', async ({
  page,
}) => {
  await openRgn(page, { spend: { cap: 1, threshold: 10 } });
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');
  await page.getByTestId('rgn-panel-submit').click();
  const dlg = page.getByTestId('rgn-confirm');
  await expect(dlg).toHaveAttribute('data-over-cap', 'true');
  await expect(dlg.getByTestId('rgn-confirm-over-cap')).toContainText('over the cap of 1 CU');
  await expect(dlg.getByTestId('rgn-confirm-ok')).toBeDisabled();
  await page.keyboard.press('Enter');
  expect(await inputsOf(page, 'rgn_job_submit')).toHaveLength(0);
  await dlg.getByTestId('rgn-confirm-cap').fill('5');
  await dlg.getByTestId('rgn-confirm-ok').click();
  await expect(dlg).toBeHidden();
  expect(await inputsOf(page, 'rgn_spend_set')).toEqual([{ cap: 5, threshold: 10 }]);
  expect(await inputsOf(page, 'rgn_job_submit')).toHaveLength(1);
  await expect(page.getByTestId('rgn-panel-spend')).toContainText('Session 2 CU of 5 CU cap');
});

test('FR-RGN-15 above the threshold Enter never confirms', async ({ page }) => {
  await openRgn(page, { spend: { cap: 100, threshold: 1 } });
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');
  await page.getByTestId('rgn-panel-submit').click();
  const dlg = page.getByTestId('rgn-confirm');
  await expect(dlg).toHaveAttribute('data-over-threshold', 'true');
  await dlg.getByTestId('rgn-confirm-second').click();
  await dlg.getByTestId('rgn-confirm-ok').focus();
  await page.keyboard.press('Enter');
  await expect(dlg).toBeVisible();
  expect(await inputsOf(page, 'rgn_job_submit')).toHaveLength(0);
  await dlg.getByTestId('rgn-confirm-ok').click();
  await expect(dlg).toBeHidden();
  expect((await inputsOf(page, 'rgn_job_submit'))[0]!.confirmations).toEqual(['threshold']);
});

test('FR-RGN-19 cancel a running job', async ({ page }) => {
  await openRgn(page);
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');
  await page.getByTestId('rgn-panel-submit').click();
  await page.getByTestId('rgn-confirm-ok').click();
  await drive(page, 'run', 0.2);
  await expect(page.getByTestId('rgn-job-state')).toHaveText('Running');
  await page.getByTestId('rgn-job-cancel').click();
  await expect(page.getByTestId('rgn-job-state')).toHaveText('Canceled');
  await expect(page.getByTestId('rgn-job-cancel')).toBeHidden();
  await expect(page.getByTestId('rgn-job-retry')).toBeVisible();
  // Submit is available again: a canceled job isn't active.
  await expect(page.getByTestId('rgn-panel-submit')).toBeEnabled();
});
