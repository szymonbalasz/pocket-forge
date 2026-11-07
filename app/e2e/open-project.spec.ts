import { expect, test, type Page } from '@playwright/test';

import { calls, defaultSeed, FIXTURE_SIZE, LIBRARY, mockCore } from './mock-core';

type View = { scale: number; tx: number; ty: number };
const view = (page: Page) => page.evaluate(() => window.__pfTest!.view() as View);

test.beforeEach(async ({ page }) => {
  await mockCore(page);
  await page.goto('/');
});

test('FR-PRJ-14 FR-EDT-01 a recent project opens, shows the image fitted, and zooms at the cursor', async ({
  page,
}) => {
  const item = page.getByTestId('prj-home-recent-item');
  await expect(item).toHaveAttribute('data-record-id', defaultSeed.recent[0]!.path);
  await item.click();

  const stage = page.getByTestId('edt-canvas-stage');
  await expect(stage).toHaveAttribute('data-image-loaded', 'true');
  await expect(page.getByTestId('prj-status-save')).toHaveAttribute('data-state', 'unsaved');

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
  await expect(page.getByTestId('edt-view-zoom')).not.toHaveText(`${Math.round(v.scale * 100)} %`);

  // Drag pans; Fit restores.
  await page.mouse.down();
  await page.mouse.move(at.x + 50, at.y + 30);
  await page.mouse.up();
  expect((await view(page)).tx).toBeCloseTo(z.tx + 50, 0);
  await page.getByTestId('edt-view-fit').click();
  expect((await view(page)).scale).toBeCloseTo(v.scale, 5);

  // FR-PRJ-23: Ctrl+S saves (first save creates r0); a second save reports no changes.
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('prj-status-save')).toHaveAttribute('data-state', 'saved');
  await page.getByTestId('prj-toolbar-save').click();
  await expect(page.getByTestId('prj-status-save')).toHaveAttribute('data-state', 'unchanged');
  const names = (await calls(page)).map((c) => c.cmd);
  expect(names).toContain('project_save_revision');
});

test('FR-PRJ-03 FR-PRJ-06 importing a pasted path opens it and summarises skips', async ({
  page,
}) => {
  await page.getByTestId('prj-home-import-path').fill('D:/maps/crypt.png\nD:/maps/art.psd');
  await page.getByTestId('prj-home-import').click();
  await expect(page.getByTestId('prj-import-summary')).toHaveText(
    '1 imported · 1 skipped (unsupported)',
  );
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
  const open = (await calls(page)).find((c) => c.cmd === 'project_open');
  expect(open?.args).toEqual({ input: { path: `${LIBRARY}/Imported/crypt.png` } });
  await expect(page.getByTestId('prj-home-recent-item').first()).toHaveText('crypt.png');
});
