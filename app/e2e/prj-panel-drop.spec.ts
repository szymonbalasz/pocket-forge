// FR-PRJ-55 (#61) on the mocked core: files dragged from Explorer onto the open project's panel.
// The drag is Tauri's `tauri://drag-over` / `tauri://drag-drop` with a position in physical px;
// the panel hit-tests it against Working, References, Structure and Parts.
import { expect, test, type Page } from '@playwright/test';

import { calls, defaultSeed, emit, mockCore, openFirstCard, projectPath } from './mock-core';

const GRID = projectPath('maps', 'grid-map');

/** The centre of a test id's box, in the physical px Tauri sends. */
async function centre(page: Page, testId: string): Promise<{ x: number; y: number }> {
  const box = (await page.getByTestId(testId).boundingBox())!;
  const dpr = await page.evaluate(() => window.devicePixelRatio || 1);
  return { x: (box.x + box.width / 2) * dpr, y: (box.y + box.height / 2) * dpr };
}

const dragOver = async (page: Page, testId: string) =>
  emit(page, 'tauri://drag-over', { position: await centre(page, testId) });
const dropOn = async (page: Page, testId: string, paths: string[]) =>
  emit(page, 'tauri://drag-drop', { paths, position: await centre(page, testId) });
const dropCalls = async (page: Page) =>
  (await calls(page)).filter(
    (c) => c.cmd === 'project_folder_add' || c.cmd === 'project_base_from_file',
  );

test.beforeEach(async ({ page }) => {
  await mockCore(page, defaultSeed);
  await page.goto('/');
  await openFirstCard(page);
  await expect(page.getByTestId('prj-panel-working-file')).toHaveCount(1);
});

test('FR-PRJ-55 a drag lights the section under it; References copies files in; elsewhere does nothing', async ({
  page,
}) => {
  await dragOver(page, 'prj-panel-references');
  await expect(page.getByTestId('prj-panel')).toHaveAttribute('data-dragging', 'true');
  await expect(page.getByTestId('prj-panel-references-dropzone')).toHaveAttribute(
    'data-over',
    'true',
  );
  await expect(page.getByTestId('prj-panel-working-dropzone')).not.toHaveAttribute(
    'data-over',
    'true',
  );
  await dragOver(page, 'prj-panel-structure');
  await expect(page.getByTestId('prj-panel-structure-dropzone')).toHaveAttribute(
    'data-over',
    'true',
  );
  await expect(page.getByTestId('prj-panel-references-dropzone')).not.toHaveAttribute(
    'data-over',
    'true',
  );

  // The canvas and History are not drop targets: no accidental imports.
  await dropOn(page, 'app-slot-canvas', ['D:/art/statue.png']);
  await dropOn(page, 'prj-panel-history', ['D:/art/statue.png']);
  await expect(page.getByTestId('prj-panel')).not.toHaveAttribute('data-dragging', 'true');
  expect(await dropCalls(page)).toEqual([]);

  await dropOn(page, 'prj-panel-references', ['D:/art/statue.png', 'D:/art/missing.png']);
  await expect(page.getByTestId('prj-panel-references-file')).toHaveCount(1);
  await expect(page.getByTestId('prj-panel-references-file')).toHaveAttribute(
    'data-record-id',
    'statue.png',
  );
  await expect(page.getByTestId('prj-panel-references-drop-notice')).toHaveText(
    'Added to References: statue.png. Skipped: missing.png (not a file).',
  );
});

test('FR-PRJ-55 one image on Working becomes the bottom image as one revision; undo restores the previous one', async ({
  page,
}) => {
  // More than one file: nothing changes.
  await dropOn(page, 'prj-panel-working', ['D:/art/a.png', 'D:/art/b.png']);
  await expect(page.getByTestId('prj-panel-working-drop-notice')).toHaveText(
    'Drop one image on Working.',
  );
  expect(await dropCalls(page)).toEqual([]);

  const history = page.getByTestId('prj-panel-history-item');
  await expect(history).toHaveCount(1);
  await dropOn(page, 'prj-panel-working', ['D:/art/Lower crypt.png']);
  await expect(page.getByTestId('prj-panel-working-drop-notice')).toHaveText(
    'New image dropped: Lower crypt.png.',
  );
  await expect(history).toHaveCount(2);
  await expect(history.first()).toHaveAttribute('data-reason', 'accept');
  await expect(history.first()).toContainText('Dropped: Lower crypt.png');
  await expect(page.getByTestId('prj-panel-working-file')).toHaveCount(2);
  const snapshot = () =>
    page.evaluate(
      () =>
        (
          window as unknown as {
            __pfMockHandlers: Record<string, (i: unknown) => { image_path: string }>;
          }
        ).__pfMockHandlers.project_get!({}).image_path,
    );
  expect(await snapshot()).toBe(`${GRID}/working/Lower crypt.png`);

  // Undo = restore the parent revision: the original is the bottom image again.
  await page.getByTestId('edt-toolbar-undo').click();
  await expect(history).toHaveCount(3);
  await expect(history.first()).toHaveAttribute('data-reason', 'restore');
  expect(await snapshot()).toBe(`${GRID}/working/grid-map-512x640.png`);
  const restore = (await calls(page)).filter((c) => c.cmd === 'project_restore_revision');
  expect(restore).toHaveLength(1);
});
