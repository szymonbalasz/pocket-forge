// REF slice 6 e2e on the mocked core (reference-packs §13, region-edit FR-RGN-41–45): the
// block's selection goes with the estimate and the job, a cost-affecting tick re-estimates, the
// confirm dialog and the tracker chip show the references, and Retry restores them with
// "changed since NNN".
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { expect, test, type Page } from '@playwright/test';

import type { Capabilities, DraftReferences } from '../src/api';
import { mockRefs, refSeed } from './ref-mock';
import { drawMaskedRegion, drive, inputsOf, openRgn } from './rgn-mock';

const REFS = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../fixtures/rgn/capabilities-refs.json', import.meta.url)),
    'utf8',
  ),
) as Capabilities;
// Like capabilities-refs.json: style ≤ 2 cost-affecting, identity ≤ 1.
const SLOTS = {
  'fake-refs': [
    { name: 'styleImages', role: 'style' as const, min: 0, max: 2, cost_affecting: true },
    { name: 'characterImages', role: 'identity' as const, min: 0, max: 1 },
  ],
};

const item = (page: Page, key: string) =>
  page.getByTestId('ref-block-item').and(page.locator(`[data-record-id="${key}"]`));
const check = (page: Page, key: string) => item(page, key).getByTestId('ref-block-item-check');
const refsOf = (input: Record<string, unknown>) =>
  (input.draft as { references?: DraftReferences | null }).references ?? null;
const isOff = (refs: DraftReferences | null, key: string) =>
  !!refs?.items.some((i) => i.key === key && !i.on);

test('FR-RGN-41 FR-RGN-42 FR-RGN-45 FR-REF-26 references go with the job and come back on Retry', async ({
  page,
}) => {
  await mockRefs(
    page,
    refSeed({ slots: SLOTS, projectRefs: [{ file: 'mood-a.png' }, { file: 'mood-b.png' }] }),
  );
  await openRgn(page, {
    models: [{ id: 'fake-refs', name: 'Refs' }],
    capabilities: { 'fake-refs': REFS },
  });
  await drawMaskedRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');
  await expect(check(page, 'project:mood-a.png')).toBeChecked();
  await expect(check(page, 'project:mood-b.png')).toBeChecked();

  // FR-RGN-42: unticking a style ref (cost-affecting slot) re-estimates with the new selection.
  await check(page, 'project:mood-b.png').uncheck();
  await expect
    .poll(async () =>
      (await inputsOf(page, 'rgn_job_estimate')).some((i) =>
        isOff(refsOf(i), 'project:mood-b.png'),
      ),
    )
    .toBe(true);
  await expect(page.getByTestId('rgn-panel-estimate')).toContainText('2 CU');

  // FR-RGN-45: the confirm dialog says what is sent; FR-RGN-41: submit carries the selection.
  await page.getByTestId('rgn-panel-submit').click();
  const dlg = page.getByTestId('rgn-confirm');
  await expect(dlg.getByTestId('rgn-confirm-references')).toHaveText('with 1 reference');
  await page.keyboard.press('Enter');
  await expect(dlg).toBeHidden();
  const submit = (await inputsOf(page, 'rgn_job_submit'))[0]!;
  expect(isOff(refsOf(submit), 'project:mood-b.png')).toBe(true);

  // FR-RGN-45: the chip shows "1 ref"; its tooltip lists the file.
  await drive(page, 'finish', 2);
  const chip = page.getByTestId('rgn-tracker-refs');
  await expect(chip).toHaveText('1 ref');
  await expect(chip).toHaveAttribute('data-tooltip', 'mood-a.png · project · style');

  // The user changes the selection, mood-a is repainted in Explorer, then Retry 001.
  await check(page, 'project:mood-b.png').check();
  await page.evaluate(() => {
    const m = (window as unknown as { __pfRefMock: { projectRefs: { hash?: string }[] } })
      .__pfRefMock;
    m.projectRefs[0]!.hash = 'mood-a repainted';
  });
  await page.getByTestId('rgn-job-retry').click();
  await page.getByTestId('rgn-job-retry-ask-confirm').click();
  await expect(check(page, 'project:mood-a.png')).toBeChecked();
  await expect(check(page, 'project:mood-b.png')).not.toBeChecked();
  await expect(item(page, 'project:mood-a.png').getByTestId('ref-block-item-badge')).toHaveText(
    'changed since 001',
  );
  // The Retry selection is what the next estimate sends.
  await expect
    .poll(async () => {
      const last = (await inputsOf(page, 'rgn_job_estimate')).at(-1);
      return last ? isOff(refsOf(last), 'project:mood-b.png') : false;
    })
    .toBe(true);
});
