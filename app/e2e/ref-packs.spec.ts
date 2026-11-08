// REF packs UI e2e on the mocked core (reference-packs §13 thin e2e): rail → packs home, create →
// add → rename → delete → restore.
import { expect, test, type Page } from '@playwright/test';

import { calls, dropFiles, mockCore } from './mock-core';
import { mockRefs, refSeed } from './ref-mock';

const cards = (page: Page) => page.getByTestId('ref-home-card');
const names = (page: Page) => page.getByTestId('ref-home-card-name');
const images = (page: Page) => page.getByTestId('ref-pack-image');

async function nameDialog(page: Page, name: string) {
  await page.getByTestId('ref-dialog-name-input').fill(name);
  await page.getByTestId('ref-dialog-name-submit').click();
}

test.beforeEach(async ({ page }) => {
  await mockRefs(page, refSeed({ sources: { 'D:/art/truncated.png': { reason: 'corrupt' } } }));
  await mockCore(page);
  await page.goto('/');
  await page.getByTestId('prj-rail-references').click();
});

test('FR-REF-07 FR-REF-08 FR-REF-10 FR-REF-12 create → add → rename → delete → restore', async ({
  page,
}) => {
  await expect(names(page)).toHaveText(['armour', 'Weapons']);
  await expect(page.getByTestId('prj-rail-references')).toHaveAttribute('aria-current', 'page');
  // Walk R7: the rail says "References" in full. It fits its 52 px item in Segoe UI, the font
  // the app ships with on Windows (about 48 px); CI's Linux fallback font is wider (about
  // 57 px), so the fit is checked on Windows runs only.
  const item = page.getByTestId('prj-rail-references');
  const label = item.locator('span');
  await expect(label).toHaveText('References');
  if (process.platform === 'win32') {
    const [text, box] = await Promise.all([label.boundingBox(), item.boundingBox()]);
    expect(text!.x).toBeGreaterThanOrEqual(box!.x);
    expect(text!.x + text!.width).toBeLessThanOrEqual(box!.x + box!.width);
  }

  // + New pack "Swords" opens it, empty.
  await page.getByTestId('ref-home-new').click();
  await nameDialog(page, 'Swords');
  await expect(page.getByTestId('ref-pack-name')).toHaveText('Swords');
  await expect(page.getByTestId('ref-pack-empty')).toBeVisible();

  // Add… (typed paths: no native dialog under Playwright), then a drop with a corrupt file.
  await page.getByTestId('ref-pack-add').click();
  await page.getByTestId('ref-pack-add-paths').fill('D:/art/a.png\nD:/art/b.jpg\nD:/art/c.webp');
  await page.getByTestId('ref-pack-add-confirm').click();
  await expect(images(page)).toHaveCount(3);
  await dropFiles(page, ['D:/art/d.png', 'D:/art/e.png', 'D:/art/truncated.png']);
  await expect(images(page)).toHaveCount(5);
  await expect(page.getByTestId('ref-add-summary-text')).toHaveText(
    '2 added · 1 skipped (corrupt)',
  );

  // Role + notes, then rename.
  await images(page).first().getByTestId('ref-pack-image-role').click();
  await page.getByRole('option', { name: 'Identity' }).click();
  await expect(images(page).first().getByTestId('ref-pack-image-role')).toHaveAttribute(
    'data-value',
    'identity',
  );
  await page.getByTestId('ref-pack-notes').fill('Hilts and blades');
  await page.getByTestId('ref-pack-name').click();
  await nameDialog(page, 'Blades');
  await expect(page.getByTestId('ref-pack-name')).toHaveText('Blades');

  // Delete → Recently deleted → Restore.
  await page.getByTestId('ref-pack-delete').click();
  await page.getByTestId('ref-dialog-delete-confirm').click();
  await expect(names(page)).toHaveText(['armour', 'Weapons']);
  await expect(page.getByTestId('ref-trash-item')).toHaveCount(1);
  await page.getByTestId('ref-trash-restore').click();
  await expect(names(page)).toHaveText(['armour', 'Blades', 'Weapons']);
  await expect(page.getByTestId('ref-trash-list')).toHaveCount(0);
  await expect(
    cards(page).filter({ hasText: 'Blades' }).getByTestId('ref-home-card-notes'),
  ).toHaveText('Hilts and blades');

  const cmds = (await calls(page)).map((c) => c.cmd).filter((c) => c.startsWith('ref_'));
  for (const c of [
    'ref_pack_create',
    'ref_pack_add_images',
    'ref_pack_update',
    'ref_pack_rename',
    'ref_pack_delete',
    'ref_trash_restore',
  ])
    expect(cmds).toContain(c);
});

test('FR-REF-07 a folder dropped on the packs home becomes a pack; the context menu renames it', async ({
  page,
}) => {
  await expect(cards(page)).toHaveCount(2);
  await dropFiles(page, ['D:/art/Crypt moods']);
  await expect(names(page)).toHaveText(['armour', 'Crypt moods', 'Weapons']);
  await cards(page).filter({ hasText: 'Crypt moods' }).click({ button: 'right' });
  await page.getByTestId('ref-home-card-rename').click();
  await nameDialog(page, 'Crypt');
  await expect(names(page)).toHaveText(['armour', 'Crypt', 'Weapons']);
});
