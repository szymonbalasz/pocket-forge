// #63 context menus on the mocked core (EXP §13 "#63", asset-project §13 (59)): the canvas menu's
// Export as and Save as with the toast, Replace and Cancel; the read-only reasons; the card,
// panel-name, file, Results and picker menus; Open in Explorer through the fake shell seam.
import { expect, test, type Page } from '@playwright/test';

import { exportState, openExport, PROJECT_DIR } from './export-mock';
import { hook, openMap } from './edt-mock';
import { LIBRARY, mockCore, projectPath } from './mock-core';
import { mockRefs } from './ref-mock';

const toast = (page: Page) => page.locator('[data-testid="exp-toast"][data-source="menu"]');

async function canvasMenu(page: Page) {
  await page
    .getByTestId('edt-canvas-stage')
    .click({ button: 'right', position: { x: 200, y: 200 } });
  await expect(page.getByTestId('edt-canvas-menu')).toBeVisible();
}

async function exportAs(page: Page, menu: string, format: 'png' | 'jpeg' | 'webp') {
  await page.getByTestId(`${menu}-export`).click();
  await page.getByTestId(`${menu}-export-${format}`).click();
}

const reveals = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { __pfPrjMock: { reveals: string[] } }).__pfPrjMock.reveals,
  );

test('FR-EXP-30 the canvas menu exports as JPEG once: the panel keeps PNG and the toast names the file', async ({
  page,
}) => {
  await openExport(page);
  await canvasMenu(page);
  for (const id of ['save-as', 'export', 'reveal'])
    await expect(page.getByTestId(`edt-canvas-menu-${id}`)).toBeVisible();
  await exportAs(page, 'edt-canvas-menu', 'jpeg');
  await expect(page.getByTestId('exp-toast-path')).toHaveText(`${PROJECT_DIR}/exports/Crypt.jpg`);
  const st = await exportState(page);
  expect(st.starts.at(-1)).toMatchObject({ replace: false, format_override: 'jpeg' });
  expect(st.starts.at(-1)!.settings.format).toBe('png');
  // The panel's settings didn't change.
  await page.keyboard.press('Control+e');
  await expect(page.getByTestId('exp-panel-format-png')).toBeChecked();
  await expect(page.getByTestId('exp-panel-start')).toBeEnabled();
});

test('FR-EXP-30 an existing file offers Replace in the toast, and Cancel stops a running one', async ({
  page,
}) => {
  // The file is there already; the run is long, so only Cancel ends it.
  await openExport(page, {
    stepMs: 60_000,
    existing: [{ path: 'exports/Crypt.webp', bytes: 12_300_000, ageMs: 2 * 86_400_000 }],
  });
  await canvasMenu(page);
  await exportAs(page, 'edt-canvas-menu', 'webp');
  await expect(page.getByTestId('exp-toast-question')).toHaveText(
    'Replace Crypt.webp (11.7 MB, 2 days ago)?',
  );
  await page.getByTestId('exp-toast-replace').click();
  await expect(toast(page)).toHaveAttribute('data-phase', 'running');
  // Start reads busy while the menu export runs (FR-EXP-33).
  await page.keyboard.press('Control+e');
  await expect(page.getByTestId('exp-panel-start')).toHaveText('Busy');
  await page.getByTestId('exp-toast-cancel').click();
  await expect(toast(page)).toHaveCount(0);
  await expect(page.getByTestId('exp-panel-start')).not.toHaveText('Busy');
  expect((await exportState(page)).starts.at(-1)).toMatchObject({ replace: true });
});

test('FR-EXP-31 Save as goes through the save dialog (mocked api seam) and writes the picked path', async ({
  page,
}) => {
  await openExport(page);
  await page.evaluate(() => {
    const w = window as unknown as Record<string, unknown>;
    const h = w.__pfMockHandlers as Record<string, (i: Record<string, unknown>) => unknown>;
    w.__pfSaveDialog = [];
    h['plugin:dialog|save'] = (i) => {
      (w.__pfSaveDialog as unknown[]).push(i.options);
      return 'D:/out/vault.webp';
    };
  });
  await canvasMenu(page);
  await page.getByTestId('edt-canvas-menu-save-as').click();
  await expect(page.getByTestId('exp-toast-path')).toHaveText('D:/out/vault.webp');
  const opts = await page.evaluate(
    () => (window as unknown as { __pfSaveDialog: { defaultPath: string }[] }).__pfSaveDialog,
  );
  expect(opts[0]!.defaultPath).toBe(`${PROJECT_DIR}/exports/Crypt.png`);
  expect((await exportState(page)).starts.at(-1)).toMatchObject({
    replace: true,
    dest: 'D:/out/vault.webp',
  });
});

test('FR-EXP-30 FR-EXP-31 on a read-only project Export as is disabled with the reason; Save as still works', async ({
  page,
}) => {
  await openExport(page, { readOnly: 'original_changed' });
  await canvasMenu(page);
  await expect(page.getByTestId('edt-canvas-menu-export')).toHaveAttribute(
    'data-reason',
    'The project is read-only',
  );
  await expect(page.getByTestId('edt-canvas-menu-save-as')).not.toHaveAttribute(
    'data-disabled',
    /.*/,
  );
});

test('FR-PRJ-59 Open in Explorer from the card, the canvas, the panel name and a file; the image menus offer Save as and Export as', async ({
  page,
}) => {
  const dir = projectPath('maps', 'grid-map');
  await mockCore(page, {
    configured: true,
    image: { w: 512, h: 640 },
    projects: [
      {
        section: 'maps',
        name: 'grid-map',
        original: 'grid-map-512x640.png',
        iterations: 0,
        last_edited: '2026-11-07T10:00:00.000Z',
        folders: {
          references: [
            { name: 'statue.png', bytes: 2048, mtime: '2026-11-01T00:00:00Z', image: true },
          ],
        },
      },
    ],
  });
  await page.goto('/');
  await page.getByTestId('prj-home-card').click({ button: 'right' });
  await page.getByTestId('prj-home-card-reveal').click();
  await expect.poll(() => reveals(page)).toEqual([dir]);

  await page.getByTestId('prj-home-card').getByRole('button').first().click();
  await expect(page.getByTestId('prj-panel')).toBeVisible();
  await page.getByTestId('prj-panel-name').click({ button: 'right' });
  await page.getByTestId('prj-panel-menu-reveal').click();
  // The canvas opens the project folder (EDT §7).
  await canvasMenu(page);
  await page.getByTestId('edt-canvas-menu-reveal').click();
  const file = page.getByTestId('prj-panel-references-file');
  await file.click({ button: 'right' });
  for (const id of ['save-as', 'export', 'reveal'])
    await expect(page.getByTestId(`prj-panel-file-menu-${id}`)).toBeVisible();
  await page.getByTestId('prj-panel-file-menu-reveal').click();
  await expect
    .poll(() => reveals(page))
    .toEqual([dir, dir, dir, `/select,${dir}/references/statue.png`]);
  expect(LIBRARY).toBe('C:/pf-e2e/library');
});

test('FR-EXP-32 a picker candidate’s menu exports its file into exports/, and twice offers Replace', async ({
  page,
}) => {
  await openMap(page);
  const dir = projectPath('maps', 'grid-map');
  await hook(page, 'setCandidates', [
    {
      id: 'C1',
      index: 1,
      status: 'ready',
      starred: false,
      thumbUrl: 'http://pf-tiles.localhost/i/th1.png',
      preview: null,
      file: `${dir}/results/001 crack/01.png`,
    },
  ]);
  const item = page.getByTestId('edt-picker-item');
  await item.click({ button: 'right' });
  await expect(page.getByTestId('edt-picker-menu-save-as')).toBeVisible();
  await exportAs(page, 'edt-picker-menu', 'webp');
  await expect(page.getByTestId('exp-toast-path')).toHaveText(
    `${dir}/exports/grid-map_001-01.webp`,
  );
  await item.click({ button: 'right' });
  await exportAs(page, 'edt-picker-menu', 'webp');
  await expect(page.getByTestId('exp-toast-question')).toContainText(
    'Replace grid-map_001-01.webp',
  );
  await page.getByTestId('exp-toast-cancel').click();
  await expect(toast(page)).toHaveCount(0);
});

test('FR-EXP-32 a pack image’s Save as runs with no project open and its toast shows on the packs home', async ({
  page,
}) => {
  await mockRefs(page);
  await mockCore(page);
  await page.addInitScript(() => {
    const h = ((window as unknown as Record<string, unknown>).__pfMockHandlers ??= {}) as Record<
      string,
      () => unknown
    >;
    h['plugin:dialog|save'] = () => 'D:/out/sword.png';
  });
  await page.goto('/');
  await page.getByTestId('prj-rail-references').click();
  await page
    .getByTestId('ref-home-card')
    .filter({ hasText: 'Weapons' })
    .getByRole('button')
    .click();
  const img = page.getByTestId('ref-pack-image').first();
  await img.click({ button: 'right' });
  await expect(page.getByTestId('ref-pack-image-menu-export')).toHaveCount(0);
  await page.getByTestId('ref-pack-image-menu-save-as').click();
  await expect(page.getByTestId('exp-toast-path')).toHaveText('D:/out/sword.png');
  const started = await page.evaluate(
    () =>
      (window as unknown as { __pfPrjMock: { fileExports: { source: string; dest: string }[] } })
        .__pfPrjMock.fileExports,
  );
  expect(started).toHaveLength(1);
  expect(started[0]!.dest).toBe('D:/out/sword.png');
  expect(started[0]!.source.startsWith(`${LIBRARY}/References/Weapons/`)).toBe(true);
});
