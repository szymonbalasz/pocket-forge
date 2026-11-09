// GEN thin e2e on the mocked core (new-asset-generation §13; slice 5, UI on mocked commands):
// + New menu → Generate new… → name → the project opens on the New image tab → size, prompt →
// estimate → confirm → the job runs in RGN's tracker with the `new image` badge. Pick and undo
// come with slice 4.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { expect, test } from '@playwright/test';

import type { Capabilities } from '../src/api';
import { installGenMock } from '../src/api/mocks/gen';
import { installRgnMock } from '../src/api/mocks/rgn';
import { MAP } from './edt-mock';
import { defaultSeed, mockCore } from './mock-core';
import { drive, inputsOf, rgnSeed } from './rgn-mock';

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
  await mockCore(page, { ...defaultSeed, image: MAP });
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

  await page.getByTestId('gen-panel-prompt').fill('flooded crypt, torchlight');
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
