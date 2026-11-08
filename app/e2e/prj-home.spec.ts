// PRJ UI e2e on the mocked core (asset-project §7, §13 thin e2e): rail → home grid, drop → summary
// → cards, open, rename, user-folder refresh, status bar.
import { expect, test, type Page } from '@playwright/test';

import {
  calls,
  defaultSeed,
  dropFiles,
  FIXTURE_SIZE,
  mockCore,
  openFirstCard,
  projectPath,
  type MockSeed,
} from './mock-core';

type View = { scale: number; tx: number; ty: number };
// The hook installs after a dynamic import (FR-EDT-26): wait for it, as edt-mock.ts does.
const view = async (page: Page) => {
  await page.waitForFunction(() => !!window.__pfTest?.view);
  return page.evaluate(() => window.__pfTest!.view!() as View);
};

const seed: MockSeed = {
  ...defaultSeed,
  projects: [
    ...defaultSeed.projects,
    { section: 'maps', name: 'Crypt', iterations: 2, last_edited: '2026-11-06T10:00:00.000Z' },
    { section: 'tokens', name: 'Goblin', last_edited: '2026-11-05T10:00:00.000Z' },
  ],
};

const cards = (page: Page) => page.getByTestId('prj-home-card');
const card = (page: Page, name: string) =>
  cards(page).filter({
    has: page.getByTestId('prj-home-card-name').getByText(name, { exact: true }),
  });

test.beforeEach(async ({ page }) => {
  await mockCore(page, seed);
  await page.goto('/');
});

test('FR-PRJ-49 section switching: the rail picks the section; Items is empty; the rail goes home from an open project', async ({
  page,
}) => {
  await expect(cards(page)).toHaveCount(2);
  await expect(page.getByTestId('prj-rail-maps')).toHaveAttribute('aria-current', 'page');
  await expect(page.getByTestId('prj-home-card-name')).toHaveText(['grid-map', 'Crypt']);

  await page.getByTestId('prj-rail-tokens').click();
  await expect(page.getByTestId('prj-home-title')).toHaveText('Tokens');
  await expect(page.getByTestId('prj-home-card-name')).toHaveText(['Goblin']);

  await page.getByTestId('prj-rail-items').click();
  await expect(page.getByTestId('prj-home-empty')).toContainText('Drop an image to start a item');
  await expect(cards(page)).toHaveCount(0);

  await page.getByTestId('prj-rail-maps').click();
  await openFirstCard(page);
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  await expect(page.getByTestId('prj-home-dropzone')).toHaveCount(0);
  await page.getByTestId('prj-rail-tokens').click();
  await expect(page.getByTestId('prj-home-card-name')).toHaveText(['Goblin']);
  await expect(page.getByTestId('edt-canvas-stage')).toHaveCount(0);
  const names = (await calls(page)).map((c) => c.cmd);
  expect(names).toContain('project_close');
});

test('FR-PRJ-03 FR-PRJ-06 FR-PRJ-08 dropping images creates cards, summarises skips, and a second drop is already in library', async ({
  page,
}) => {
  await expect(cards(page)).toHaveCount(2);
  await expect(page.getByTestId('prj-home-dropzone')).toBeVisible();
  await dropFiles(page, ['D:/maps/Sewer.png', 'D:/maps/cave.jpg', 'D:/maps/art.psd']);
  await expect(page.getByTestId('prj-import-summary-text')).toHaveText(
    '2 created · 1 skipped (unsupported)',
  );
  await expect(cards(page)).toHaveCount(4);
  await expect(card(page, 'Sewer')).toHaveCount(1);
  await expect(page.getByTestId('prj-import-summary-item')).toHaveAttribute(
    'data-status',
    'skipped',
  );
  const create = (await calls(page)).find((c) => c.cmd === 'project_create');
  expect(create?.args).toEqual({
    input: { section: 'maps', paths: ['D:/maps/Sewer.png', 'D:/maps/cave.jpg', 'D:/maps/art.psd'] },
  });

  await dropFiles(page, ['D:/maps/Sewer.png']);
  await expect(page.getByTestId('prj-import-summary-text')).toHaveText('1 already in library');
  await page.getByTestId('prj-import-summary-open').click();
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  await expect(page.getByTestId('prj-panel-name')).toHaveText('Sewer');
  const open = (await calls(page)).filter((c) => c.cmd === 'project_open').at(-1);
  expect(open?.args).toEqual({ input: { path: projectPath('maps', 'Sewer') } });
});

test('FR-PRJ-14 FR-EDT-01 a card opens the project folder, fits the image and zooms at the cursor; FR-PRJ-23 Save', async ({
  page,
}) => {
  const first = cards(page).first();
  await expect(first).toHaveAttribute('data-record-id', /^01JMOCKPRJ/);
  await openFirstCard(page);
  const stage = page.getByTestId('edt-canvas-stage');
  await expect(stage).toHaveAttribute('data-image-loaded', 'true');
  await expect(page.getByTestId('prj-status-save')).toHaveAttribute('data-state', 'saved');
  const open = (await calls(page)).find((c) => c.cmd === 'project_open');
  expect(open?.args).toEqual({ input: { path: projectPath('maps', 'grid-map') } });

  // FR-EDT-01: fitted and centred.
  const box = (await stage.boundingBox())!;
  const v = await view(page);
  const w = FIXTURE_SIZE.w * v.scale;
  const h = FIXTURE_SIZE.h * v.scale;
  expect(Math.min(box.width / FIXTURE_SIZE.w, box.height / FIXTURE_SIZE.h)).toBeCloseTo(v.scale, 5);
  expect(v.tx + w / 2).toBeCloseTo(box.width / 2, 0);
  expect(v.ty + h / 2).toBeCloseTo(box.height / 2, 0);

  // FR-EDT-02: wheel zoom keeps the image point under the cursor.
  const at = { x: box.x + box.width * 0.4, y: box.y + box.height * 0.6 };
  const before = { x: (at.x - box.x - v.tx) / v.scale, y: (at.y - box.y - v.ty) / v.scale };
  await page.mouse.move(at.x, at.y);
  await page.mouse.wheel(0, -300);
  await expect.poll(async () => (await view(page)).scale).toBeGreaterThan(v.scale);
  const z = await view(page);
  const after = { x: (at.x - box.x - z.tx) / z.scale, y: (at.y - box.y - z.ty) / z.scale };
  expect(Math.hypot(after.x - before.x, after.y - before.y) * z.scale).toBeLessThan(0.5);

  // FR-PRJ-23: Ctrl+S with nothing changed → no changes; a labelled Save adds r1.
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('prj-status-save')).toHaveAttribute('data-state', 'unchanged');
  await page.getByTestId('prj-panel-history-label').fill('first pass');
  await page.getByTestId('prj-panel-history-save').click();
  await expect(page.getByTestId('prj-status-save')).toHaveAttribute('data-state', 'saved');
  await expect(page.getByTestId('prj-panel-history-item')).toHaveCount(2);
  await expect(page.getByTestId('prj-panel-history-item').first()).toContainText('first pass');
});

test('FR-PRJ-45 rename from the card context menu renames the folder; a taken name is refused', async ({
  page,
}) => {
  await card(page, 'Crypt').click({ button: 'right' });
  await page.getByTestId('prj-home-card-rename').click();
  const input = page.getByTestId('prj-home-card-rename-input');
  await expect(input).toBeFocused();
  await input.fill('GRID-MAP');
  await input.press('Enter');
  await expect(page.getByTestId('prj-home-card-rename-error')).toContainText('taken');
  await input.fill('Old Crypt');
  await input.press('Enter');
  await expect(card(page, 'Old Crypt')).toHaveCount(1);
  await expect(card(page, 'Crypt')).toHaveCount(0);
  const renames = (await calls(page)).filter((c) => c.cmd === 'project_rename');
  expect(renames.at(-1)?.args).toEqual({
    input: { path: projectPath('maps', 'Crypt'), name: 'Old Crypt' },
  });
  // The renamed project opens from its new folder.
  await card(page, 'Old Crypt').getByRole('button').click();
  await expect(page.getByTestId('prj-panel-name')).toHaveText('Old Crypt');
});

test('FR-PRJ-48 a file copied into References in Explorer shows on Refresh; one in Parts on window focus', async ({
  page,
}) => {
  await openFirstCard(page);
  await expect(page.getByTestId('prj-panel-working-file')).toHaveAttribute(
    'data-record-id',
    'grid-map-512x640.png',
  );
  await expect(page.getByTestId('prj-panel-references-file')).toHaveCount(0);
  const addInExplorer = (folder: string, name: string) =>
    page.evaluate(
      ([p, f, n]) =>
        (
          window as unknown as {
            __pfPrjMock: { addUserFile: (p: string, f: string, x: unknown) => void };
          }
        ).__pfPrjMock.addUserFile(p!, f!, {
          name: n,
          bytes: 4096,
          mtime: '2026-11-08T12:00:00Z',
          image: true,
        }),
      [projectPath('maps', 'grid-map'), folder, name],
    );
  await addInExplorer('references', 'mood.jpg');
  await page.getByTestId('prj-panel-references-refresh').click();
  await expect(page.getByTestId('prj-panel-references-file')).toHaveAttribute(
    'data-record-id',
    'mood.jpg',
  );
  await addInExplorer('parts', 'door.png');
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(page.getByTestId('prj-panel-parts-file')).toHaveAttribute(
    'data-record-id',
    'door.png',
  );
});

test('R40 the provider chip is a status-bar segment next to the PRJ save state; no rail chip', async ({
  page,
}) => {
  await openFirstCard(page);
  await expect(
    page.locator('[data-segment="provider:chip"]').getByTestId('prv-status-chip'),
  ).toBeVisible();
  await expect(
    page.locator('[data-segment="project:save"]').getByTestId('prj-status-save'),
  ).toBeVisible();
  await expect(page.getByTestId('prv-status-mode')).toHaveCount(0);
  await expect(page.getByTestId('app-slot-rail').getByTestId('prv-status-chip')).toHaveCount(0);
});

test('FR-PRJ-03 #53 + New uses the native image dialog when it is available', async ({ page }) => {
  await page.evaluate(() => {
    const h = (window as unknown as { __pfMockHandlers: Record<string, () => unknown> })
      .__pfMockHandlers;
    h['plugin:dialog|open'] = () => ['D:/maps/Bridge.png', 'D:/maps/Ford.webp'];
  });
  await expect(cards(page)).toHaveCount(2);
  await page.getByTestId('prj-home-new').click();
  await expect(page.getByTestId('prj-import-summary-text')).toHaveText('2 created');
  await expect(cards(page)).toHaveCount(4);
  await expect(page.getByTestId('prj-home-new-paths')).toHaveCount(0);
});
