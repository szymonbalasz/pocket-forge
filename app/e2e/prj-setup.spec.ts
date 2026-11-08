// PRJ library root on the mocked core: first run (FR-PRJ-01) and an unavailable root (FR-PRJ-02).
import { expect, test } from '@playwright/test';

import { calls, defaultSeed, dropFiles, LIBRARY, mockCore } from './mock-core';

test.describe('first run', () => {
  test.beforeEach(async ({ page }) => {
    await mockCore(page, { ...defaultSeed, configured: false });
    await page.goto('/');
  });

  test('FR-PRJ-01 choose the library folder: a field error, then the Maps home shows', async ({
    page,
  }) => {
    await page.getByTestId('prj-setup-root-path').fill('E:/missing');
    await page.getByTestId('prj-setup-pick-root').click();
    await expect(page.getByTestId('prj-setup-error')).toHaveText('That folder does not exist');
    await page.getByTestId('prj-setup-root-path').fill(LIBRARY);
    await page.getByTestId('prj-setup-pick-root').click();
    await expect(page.getByTestId('prj-home-title')).toHaveText('Maps');
    await expect(page.getByTestId('prj-home-card')).toHaveCount(1);
  });
});

test.describe('library unavailable', () => {
  test.beforeEach(async ({ page }) => {
    await mockCore(page, { ...defaultSeed, prj: { unavailable: true } });
    await page.goto('/');
  });

  test('FR-PRJ-02 an unavailable root shows the banner with Choose again, and a drop creates nothing', async ({
    page,
  }) => {
    await expect(page.getByTestId('prj-library-unavailable')).toContainText(LIBRARY);
    await expect(page.getByTestId('prj-home-new')).toBeDisabled();
    await dropFiles(page, ['D:/maps/x.png']);
    await expect(page.getByTestId('prj-home-card')).toHaveCount(0);
    // The core refuses it (`library_unavailable`): nothing is written under the root.
    await expect(page.getByTestId('prj-error')).toHaveAttribute('data-code', 'library_unavailable');
    expect((await calls(page)).some((c) => c.cmd === 'project_create')).toBe(true);
    await page.getByTestId('prj-library-choose-again').click();
    await expect(page.getByTestId('prj-setup')).toBeVisible();
  });
});
