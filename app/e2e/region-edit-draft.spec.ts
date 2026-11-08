// RGN draft on open (asset-project walk D2, FR-PRJ-22, FR-RGN-11): opening an asset project
// shows its saved draft and never writes it; the draft is per project.
import { expect, test, type Page } from '@playwright/test';

import { MAP, openMap } from './edt-mock';
import { defaultSeed, openFirstCard } from './mock-core';
import { drawRegion, inputsOf, rgnSeed } from './rgn-mock';
import { installRgnMock } from '../src/api/mocks/rgn';

/** region-edit/store.ts DRAFT_SAVE_MS (the store isn't importable outside the app). */
const DRAFT_SAVE_MS = 2000;
const drafts = (page: Page) => inputsOf(page, 'project_put_draft');

/** The rgn mock keeps one draft for the whole library; give each project its own. */
async function draftPerProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    type Rec = Record<string, unknown>;
    const w = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args: Rec) => Promise<unknown> };
      __pfMockSnapshot: Rec;
      __pfMock: { calls: { cmd: string; args: { input?: Rec } }[] };
    };
    const blank = {
      final_prompt: '',
      model_id: null,
      controls: {},
      seed: null,
      region: null,
      mask_id: null,
    };
    const store: Record<string, unknown> = {};
    let open = String(
      w.__pfMock.calls.filter((c) => c.cmd === 'project_open').at(-1)?.args.input?.path,
    );
    const invoke = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = (cmd, args) => {
      if (cmd === 'project_open') {
        store[open] = w.__pfMockSnapshot.draft;
        open = String((args.input as Rec).path);
        w.__pfMockSnapshot.draft = store[open] ?? { ...blank };
      }
      return invoke(cmd, args);
    };
  });
}

async function openRgnMaps(page: Page, names: string[]): Promise<void> {
  await page.addInitScript(installRgnMock, rgnSeed());
  const projects = names.map((name, i) => ({
    ...defaultSeed.projects[0]!,
    name,
    last_edited: `2026-11-07T1${9 - i}:00:00.000Z`, // newest first: names[0] is the first card
  }));
  await openMap(page, { seed: { ...defaultSeed, image: MAP, projects } });
  await expect(page.getByTestId('rgn-panel')).toBeVisible();
}

async function closeProject(page: Page): Promise<void> {
  await page.getByTestId('prj-panel-close').click();
  await expect(page.getByTestId('prj-home-card').first()).toBeVisible();
}

test('FR-PRJ-22 FR-RGN-11 reopening keeps the saved prompt and region and writes no draft (D2a)', async ({
  page,
}) => {
  await openRgnMaps(page, ['crypt']);
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('rgn-panel-prompt').fill('flooded crypt');
  await expect
    .poll(async () => (await drafts(page)).at(-1)?.draft, { timeout: DRAFT_SAVE_MS + 3000 })
    .toMatchObject({ final_prompt: 'flooded crypt', region: { inner: { w: 400, h: 400 } } });

  await closeProject(page);
  const before = (await drafts(page)).length;
  await openFirstCard(page);
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  await expect(page.getByTestId('rgn-panel-prompt')).toHaveValue('flooded crypt');
  await expect(page.getByTestId('rgn-panel-size')).toHaveText('400 × 400 px');
  await page.waitForTimeout(DRAFT_SAVE_MS + 500);
  expect((await drafts(page)).slice(before)).toEqual([]);
  await expect(page.getByTestId('rgn-panel-prompt')).toHaveValue('flooded crypt');
});

test("FR-PRJ-22 opening another project never copies the previous project's region (D2b)", async ({
  page,
}) => {
  await openRgnMaps(page, ['hall', 'cellar']);
  await draftPerProject(page);
  await drawRegion(page, [400, 500], [800, 900]);
  await expect
    .poll(async () => (await drafts(page)).at(-1)?.draft, { timeout: DRAFT_SAVE_MS + 3000 })
    .toMatchObject({ region: { inner: { w: 400, h: 400 } } });

  await closeProject(page);
  const before = (await drafts(page)).length;
  await page.getByTestId('prj-home-card').nth(1).getByRole('button').click();
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  await expect(page.getByTestId('rgn-panel-empty')).toBeVisible();
  await page.waitForTimeout(DRAFT_SAVE_MS + 500);
  expect((await drafts(page)).slice(before)).toEqual([]);
  await expect(page.getByTestId('rgn-panel-empty')).toBeVisible();
});
