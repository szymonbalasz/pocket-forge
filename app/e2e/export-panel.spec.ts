// EXP slice 4: the Export panel on the mocked core (KQ-33, EXP §13 thin e2e).
import { expect, test, type Page } from '@playwright/test';

import { calls, emit } from './mock-core';
import { exportState, expandExport, openExport, PROJECT_DIR } from './export-mock';

const plan = (page: Page) => page.getByTestId('exp-panel-plan');
const start = (page: Page) => page.getByTestId('exp-panel-start');

async function setCells(page: Page, cols: string, rows: string, target?: string) {
  await page.getByTestId('exp-panel-grid-mode-cells').click();
  await page.getByTestId('exp-panel-grid-cols').fill(cols);
  await page.getByTestId('exp-panel-grid-rows').fill(rows);
  if (target !== undefined) await page.getByTestId('exp-panel-target-ppsq').fill(target);
}

test('FR-EXP-06 FR-EXP-07 FR-EXP-08 FR-EXP-26 grid + target → plan line, and EDT display size turns on', async ({
  page,
}) => {
  await openExport(page);
  await expect(page.getByTestId('exp-panel-grid-mode')).toBeHidden();
  await expandExport(page);
  // Ctrl+E focuses the section (EXP §7).
  await expect(page.getByTestId('exp-panel-grid-mode-px')).toBeFocused();
  await expect(plan(page)).toHaveText('4200×5600 · Full resolution · exports/Crypt.png');
  await expect(page.getByTestId('exp-panel-target-ppsq')).toBeDisabled();
  await expect(page.getByTestId('edt-view-display')).toBeDisabled();

  await setCells(page, '30', '40');
  await expect(page.getByTestId('exp-panel-target-ppsq')).toBeEnabled();
  await expect(page.getByTestId('edt-view-display')).toBeDisabled(); // needs both (R6)
  await page.getByTestId('exp-panel-target-ppsq').fill('70');
  await expect(plan(page)).toHaveText('4200×5600 → 2100×2800 · 0.5× · exports/Crypt_70px.png');
  await expect(page.getByTestId('edt-view-display')).toBeEnabled();
  await page.getByTestId('edt-view-display').click();
  await expect(page.getByTestId('edt-view-zoom')).toHaveText('50 %');

  // Non-square grid warns without blocking; an upscale warns.
  await page.getByTestId('exp-panel-grid-rows').fill('39');
  await expect(page.getByTestId('exp-panel-warnings')).toContainText("Grid isn't square");
  await expect(start(page)).toBeEnabled();
  await page.getByTestId('exp-panel-grid-rows').fill('40');
  await page.getByTestId('exp-panel-target-ppsq').fill('280');
  await expect(page.getByTestId('exp-panel-warnings')).toContainText('Upscaled, softer');

  // Limits from invalid_settings land on the field and block Start.
  await page.getByTestId('exp-panel-grid-cols').fill('1001');
  await expect(page.getByTestId('exp-panel-grid-cols-error')).toHaveText('At most 1000');
  await expect(start(page)).toBeDisabled();
  await page.getByTestId('exp-panel-grid-cols').fill('30');
  await page.getByTestId('exp-panel-target-ppsq').fill('500'); // 3.6×: 5600 → 20000 px
  await expect(page.getByTestId('exp-panel-target-ppsq-error')).toHaveText(
    'Output would be over 16384 px',
  );

  // Typing never starts or commits an export (FR-EXP-25).
  expect((await calls(page)).some((c) => c.cmd === 'export_start')).toBe(false);
});

test('FR-EXP-19 FR-EXP-27 Start → progress → done toast with copy path, then Replace (FR-EXP-15)', async ({
  page,
}) => {
  await openExport(page, { steps: 8, stepMs: 120 });
  await expandExport(page);
  await setCells(page, '30', '40', '70');
  await expect(plan(page)).toContainText('exports/Crypt_70px.png');
  await expect(start(page)).toHaveText('Export');
  await start(page).click();

  await expect(page.getByTestId('exp-panel-progress')).toBeVisible();
  await expect(page.getByTestId('exp-status-progress')).toContainText('Exporting');
  await expect(page.getByTestId('exp-panel-cancel')).toBeEnabled();
  // The canvas stays usable while it runs.
  await page.getByTestId('edt-view-fit').click();

  const toast = page.getByTestId('exp-toast');
  await expect(toast).toContainText('Exported Crypt_70px.png');
  await expect(page.getByTestId('exp-toast-detail')).toContainText('2100×2800');
  await expect(page.getByTestId('exp-panel-progress')).toBeHidden();
  await page.getByTestId('exp-toast-copy').click();
  await expect(page.getByTestId('exp-toast-copy')).toHaveText('Copied');
  expect(
    await page.evaluate(() => (window as unknown as { __pfClipboard: string[] }).__pfClipboard),
  ).toEqual([`${PROJECT_DIR}/exports/Crypt_70px.png`]);

  // The file exists now: Replace, with the note, and replace: true (no modal).
  await expect(start(page)).toHaveText('Replace');
  await expect(page.getByTestId('exp-panel-replace-note')).toHaveText(
    /^Replaces Crypt_70px\.png \(\d+(\.\d)? MB, now\)$/,
  );
  // The toast sits below the section's controls, so Replace works while it shows (D1).
  await expect(toast).toBeVisible();
  const [toastBox, startBox, noteBox] = await Promise.all([
    toast.boundingBox(),
    start(page).boundingBox(),
    page.getByTestId('exp-panel-replace-note').boundingBox(),
  ]);
  expect(toastBox!.y).toBeGreaterThanOrEqual(startBox!.y + startBox!.height);
  expect(toastBox!.y).toBeGreaterThanOrEqual(noteBox!.y + noteBox!.height);
  await start(page).click({ timeout: 2_000 });
  await expect(page.getByTestId('exp-panel-progress')).toBeVisible();
  const st = await exportState(page);
  expect(st.starts.map((s) => s.replace)).toEqual([false, true]);
  expect(st.starts[1]!.settings).toEqual({
    grid: { mode: 'cells', source_px_per_square: null, cols: 30, rows: 40 },
    target_px_per_square: 70,
    format: 'png',
  });
});

test('FR-EXP-15 an existing file shows Replace with its size and age', async ({ page }) => {
  await openExport(page, {
    existing: [{ path: 'exports/Crypt.png', bytes: 12.3 * 1024 * 1024, ageMs: 2 * 86400_000 }],
  });
  await expandExport(page);
  await expect(start(page)).toHaveText('Replace');
  await expect(page.getByTestId('exp-panel-replace-note')).toHaveText(
    'Replaces Crypt.png (12.3 MB, 2 days ago)',
  );
  await page.getByTestId('exp-panel-format-jpeg').click();
  await expect(start(page)).toHaveText('Export');
  await expect(page.getByTestId('exp-panel-replace-note')).toBeHidden();
});

test('FR-EXP-11 WebP is disabled with its reason above 16383 px', async ({ page }) => {
  await openExport(page, { image: { w: 16384, h: 16384 } });
  await expandExport(page);
  const webp = page.getByTestId('exp-panel-format-webp');
  await expect(webp).toBeDisabled();
  await expect(page.getByTestId('exp-panel-format-png')).toBeEnabled();
  await page.getByTestId('exp-panel-format-webp-reason').hover();
  await expect(page.getByRole('tooltip')).toHaveText('WebP max 16383 px');
});

test('FR-EXP-28 Start waits for the original; FR-EXP-29 read-only disables it', async ({
  page,
}) => {
  await openExport(page, { originalReadyMs: null });
  await expandExport(page, false);
  expect((await calls(page)).filter((c) => c.cmd === 'project_prefetch_original')).toHaveLength(1);
  await expect(start(page)).toBeDisabled();
  await expect(page.getByTestId('exp-panel-start-reason')).toHaveText('Loading original…');
  await emit(page, 'project:original-ready', null);
  await expect(start(page)).toBeEnabled();
});

test('FR-EXP-29 read-only: the plan works, Start is disabled with the reason', async ({ page }) => {
  await openExport(page, { readOnly: 'original_changed' });
  await expandExport(page, false);
  await expect(page.getByTestId('exp-panel')).toHaveAttribute('data-state-panel', 'read-only');
  await setCells(page, '30', '40', '70');
  await expect(plan(page)).toHaveText('4200×5600 → 2100×2800 · 0.5× · exports/Crypt_70px.png');
  await expect(start(page)).toBeDisabled();
  await expect(page.getByTestId('exp-panel-start-reason')).toHaveText(
    'Read-only: the original image changed',
  );
});

test('FR-EXP-20 Cancel stops the export and leaves no file', async ({ page }) => {
  await openExport(page, { steps: 30, stepMs: 150 });
  await expandExport(page);
  await start(page).click();
  await expect(page.getByTestId('exp-panel-progress')).not.toHaveAttribute('aria-valuenow', '0');
  await page.getByTestId('exp-panel-cancel').click();
  await expect(page.getByTestId('exp-panel-progress')).toBeHidden();
  await expect(start(page)).toHaveText('Export');
  await expect(page.getByTestId('exp-toast')).toBeHidden();
  await expect(page.getByTestId('exp-panel-error')).toBeHidden();
  const st = await exportState(page);
  expect(st.existing).toEqual({});
  expect(st.running).toBeNull();
});

test('FR-EXP-18 a failed export shows the message, and Retry runs it again', async ({ page }) => {
  await openExport(page, { failRun: 'locked' });
  await expandExport(page);
  await start(page).click();
  await expect(page.getByTestId('exp-panel-error')).toContainText(
    'The file is in use (VTT or sync?)',
  );
  await page.getByTestId('exp-panel-retry').click();
  await expect(page.getByTestId('exp-toast')).toContainText('Exported Crypt.png');
  await expect(page.getByTestId('exp-panel-error')).toBeHidden();
});

test('FR-EXP-12 the WebP lossless fallback and an unsaved record add notices', async ({ page }) => {
  await openExport(page, { webpFallback: true, recordSaved: false });
  await expandExport(page);
  await page.getByTestId('exp-panel-format-webp').click();
  await expect(plan(page)).toContainText('exports/Crypt.webp');
  await start(page).click();
  await expect(page.getByTestId('exp-toast-fallback')).toBeVisible();
  await expect(page.getByTestId('exp-toast-unsaved')).toBeVisible();
});

test('FR-EXP-25 saved settings restore into the panel at open', async ({ page }) => {
  await openExport(page, {
    saved: {
      grid: { mode: 'cells', cols: 30, rows: 40 },
      target_px_per_square: 70,
      format: 'webp',
    },
  });
  // EDT's display size is on from export_get_settings alone (FR-EXP-26).
  await expect(page.getByTestId('edt-view-display')).toBeEnabled();
  await expandExport(page);
  await expect(page.getByTestId('exp-panel-grid-cols')).toHaveValue('30');
  await expect(page.getByTestId('exp-panel-target-ppsq')).toHaveValue('70');
  await expect(page.getByTestId('exp-panel-format-webp')).toHaveAttribute('data-state', 'checked');
  await expect(plan(page)).toHaveText('4200×5600 → 2100×2800 · 0.5× · exports/Crypt_70px.webp');
});
