// REF References block e2e on the mocked core (reference-packs §13 thin e2e): the block in the
// Generate panel, the tick limit, the pack picker, roles, and submit blocked under a slot's min.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { expect, test, type Page } from '@playwright/test';

import type { Capabilities } from '../src/api';
import { mockRefs, refSeed } from './ref-mock';
import { drawRegion, inputsOf, openRgn } from './rgn-mock';

const REFS = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../fixtures/rgn/capabilities-refs.json', import.meta.url)),
    'utf8',
  ),
) as Capabilities;
const REQUIRED: Capabilities = {
  ...REFS,
  model_id: 'fake-required',
  slots: [
    {
      name: 'characterImages',
      role: 'identity',
      min: 1,
      max: 1,
      cost_affecting: false,
      array: true,
    },
  ],
};
const WEAPONS = '01JMOCKPACKWEAPONS00000000';
const SLOTS = {
  'fake-refs': [
    { name: 'styleImages', role: 'style' as const, min: 0, max: 2 },
    { name: 'characterImages', role: 'identity' as const, min: 0, max: 1 },
  ],
  'fake-required': [{ name: 'characterImages', role: 'identity' as const, min: 1, max: 1 }],
};

const item = (page: Page, key: string) =>
  page.getByTestId('ref-block-item').and(page.locator(`[data-record-id="${key}"]`));
const check = (page: Page, key: string) => item(page, key).getByTestId('ref-block-item-check');

async function pickWeapons(page: Page) {
  await page.getByTestId('ref-block-pack-picker').click();
  await page
    .getByTestId('ref-block-pack-option')
    .and(page.locator(`[data-record-id="${WEAPONS}"]`))
    .getByRole('checkbox')
    .check();
  await page.keyboard.press('Escape');
}

test('FR-REF-18 FR-REF-19 tick limit, pack picker and roles in the Generate panel', async ({
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
  const block = page.getByTestId('ref-block');
  await expect(block).toBeVisible();
  await expect(page.getByTestId('ref-block-count')).toHaveText('Style 2/2 · Identity 0/1');
  await expect(check(page, 'project:mood-a.png')).toBeChecked();
  await expect(check(page, 'project:mood-b.png')).toBeChecked();

  await pickWeapons(page);
  const a = `pack:${WEAPONS}:swatch-a.png`;
  const b = `pack:${WEAPONS}:swatch-b.jpg`;
  await expect(item(page, a)).toBeVisible();
  await expect(check(page, a)).toBeDisabled();
  await expect(item(page, a).getByTestId('ref-block-item-limit')).toHaveText('limit 2 reached');
  await expect(page.getByTestId('ref-block-over')).toHaveText('5 available · 2 sent');

  // Move swatch-b to the identity slot, then tick it there.
  await item(page, b).getByTestId('ref-block-item-role').click();
  await page.getByRole('option', { name: 'Identity' }).click();
  await expect(item(page, b).getByTestId('ref-block-item-role')).toHaveAttribute(
    'data-value',
    'identity',
  );
  await check(page, b).check();
  await expect(page.getByTestId('ref-block-count')).toHaveText('Style 2/2 · Identity 1/1');

  // × unpicks the pack.
  await page.getByTestId('ref-block-pack-unpick').click();
  await expect(item(page, a)).toHaveCount(0);
  await expect(page.getByTestId('ref-block-count')).toHaveText('Style 2/2 · Identity 0/1');
});

test('FR-REF-18 a required slot under its min disables submit and names the slot', async ({
  page,
}) => {
  await mockRefs(page, refSeed({ slots: SLOTS, projectRefs: [] }));
  await openRgn(page, {
    models: [{ id: 'fake-required', name: 'Required' }],
    capabilities: { 'fake-required': REQUIRED },
  });
  await drawRegion(page, [400, 500], [800, 900]);
  await expect(page.getByTestId('ref-block-under')).toHaveText('Identity needs at least 1');
  await expect(page.getByTestId('rgn-panel-submit')).toBeDisabled();
  await expect(page.getByTestId('rgn-panel-submit-reason')).toContainText(
    'Identity needs at least 1',
  );

  await pickWeapons(page);
  await expect(page.getByTestId('ref-block-under')).toHaveCount(0);
  await expect(page.getByTestId('rgn-panel-submit-reason')).not.toContainText('References');
});

test('FR-REF-21 FR-RGN-45 walk R1/R2: the confirm counts the current ticks; a ticked file gone before Generate fails with no job', async ({
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
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('rgn-panel-prompt').fill('A stone well');
  await expect(page.getByTestId('rgn-panel-submit')).toBeEnabled();
  const estimates = (await inputsOf(page, 'rgn_job_estimate')).length;

  // R2: an identity tick (not cost-affecting) reuses the estimate; the confirm still counts 3.
  await pickWeapons(page);
  const b = `pack:${WEAPONS}:swatch-b.jpg`;
  await item(page, b).getByTestId('ref-block-item-role').click();
  await page.getByRole('option', { name: 'Identity' }).click();
  await check(page, b).check();
  await expect(page.getByTestId('ref-block-count')).toHaveText('Style 2/2 · Identity 1/1');
  await page.getByTestId('rgn-panel-submit').click();
  const dlg = page.getByTestId('rgn-confirm');
  await expect(dlg.getByTestId('rgn-confirm-references')).toHaveText('with 3 references');
  expect((await inputsOf(page, 'rgn_job_estimate')).length).toBe(estimates);
  await dlg.getByTestId('rgn-confirm-cancel').click();

  // R1: mood-b is moved away in Explorer, and nothing resolves the block before Generate.
  await page.evaluate(() => {
    const m = (window as unknown as { __pfRefMock: { projectRefs: { file: string }[] } })
      .__pfRefMock;
    m.projectRefs = m.projectRefs.filter((f) => f.file !== 'mood-b.png');
  });
  await page.getByTestId('rgn-panel-submit').click();
  await dlg.getByTestId('rgn-confirm-ok').click();
  await expect(page.getByTestId('rgn-panel-submit-error')).toContainText(
    "mood-b.png can't be sent: it can't be found",
  );
  expect(await inputsOf(page, 'rgn_job_submit')).toHaveLength(1);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __pfRgnMock: { state: () => { jobs: unknown[] } } }).__pfRgnMock
          .state().jobs.length,
    ),
  ).toBe(0);
  // The block resolves again and says which one.
  await expect(
    page.getByTestId('ref-block-notice').and(page.locator('[data-code="dropped"]')),
  ).toHaveText(/mood-b\.png is no longer available/);
  await expect(item(page, 'project:mood-b.png')).toHaveCount(0);
});
