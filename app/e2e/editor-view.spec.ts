// EDT slice 2: view, presets, HiDPI, tiles, shortcuts (mocked core, KQ-33). Assertions read
// `__pfTest`, not canvas pixels (ADR 0003).
import { expect, test } from '@playwright/test';

import { hook, openMap, stageBox, view } from './edt-mock';
import { calls, emit } from './mock-core';

test('FR-EDT-04 presets: actual pixels, fit, and display size enabled by export settings', async ({
  page,
}) => {
  await openMap(page);
  const fitted = await view(page);
  const display = page.getByTestId('edt-view-display');
  await expect(display).toBeDisabled();
  await expect(display.locator('..')).toHaveAttribute('data-tooltip', /needs both px-per-square/);

  await page.keyboard.press('Control+1');
  expect((await view(page)).scale).toBeCloseTo(1, 6);
  await expect(page.getByTestId('edt-view-zoom')).toHaveText('100 %');
  await page.keyboard.press('Control+0');
  expect((await view(page)).scale).toBeCloseTo(fitted.scale, 6);

  // Only one value → still disabled (R6); both → enabled.
  await emit(page, 'export:settings-changed', { source: 140, target: null });
  await expect(display).toBeDisabled();
  await emit(page, 'export:settings-changed', { source: 140, target: 70 });
  await expect(display).toBeEnabled();
  await page.keyboard.press('Control+2');
  expect((await view(page)).scale).toBeCloseTo(0.5, 6);
  await expect(page.getByTestId('edt-view-zoom')).toHaveText('50 %');
});

test('FR-EDT-04 display size reads export_get_settings at open', async ({ page }) => {
  await openMap(page, { pxPerSquare: { source: 100, target: 50 } });
  await expect(page.getByTestId('edt-view-display')).toBeEnabled();
  await page.getByTestId('edt-view-display').click();
  expect((await view(page)).scale).toBeCloseTo(0.5, 6);
});

test.describe('HiDPI', () => {
  test.use({ deviceScaleFactor: 2 });

  test('FR-EDT-05 pixel ratio and level follow DPR, and change on the next frame without reload', async ({
    page,
  }) => {
    await openMap(page);
    expect(await hook(page, 'pixelRatio')).toBe(2);
    expect(await hook(page, 'dpr')).toBe(2);
    // Actual pixels at DPR 2 = 0.5 CSS px per image px; readout says 100 %.
    await page.keyboard.press('Control+1');
    expect((await view(page)).scale).toBeCloseTo(0.5, 6);
    await expect(page.getByTestId('edt-view-zoom')).toHaveText('100 %');
    // device scale 1 → level 0.
    await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-level', '0');
    // Fit (≈ 0.3 CSS px per image px) is 0.6 device px at DPR 2 → level 0 (level 1 is 0.5).
    await page.keyboard.press('Control+0');
    const fitScale = (await view(page)).scale;
    expect(fitScale * 2).toBeGreaterThan(0.5);
    await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-level', '0');

    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width: 1280,
      height: 800,
      deviceScaleFactor: 1,
      mobile: false,
    });
    const next = await page.evaluate(
      () =>
        new Promise<{ ratio: number; dpr: number }>((resolve) =>
          requestAnimationFrame(() =>
            resolve({
              ratio: window.__pfTest!.pixelRatio!(),
              dpr: window.__pfTest!.dpr!(),
            }),
          ),
        ),
    );
    expect(next).toEqual({ ratio: 1, dpr: 1 });
    // The same CSS scale is now ≈ 0.3 device px → level 1, with no reload.
    await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-level', '1');
    await expect(page.getByTestId('edt-view-zoom')).toHaveText(`${Math.round(fitScale * 100)} %`);
  });
});

test('FR-EDT-07 FR-EDT-08 the one-tile level shows first; level 0 appears after its progress event', async ({
  page,
}) => {
  await openMap(page, { incomplete: [0] });
  const stage = page.getByTestId('edt-canvas-stage');
  const before = await hook<{ shown: string[] }>(page, 'tiles');
  expect(before.shown).toContain('http://pf-tiles.localhost/t/tk1/3/0_0.jpg');
  // At 1:1 level 0 would be chosen, but it isn't built: the finest built level (1) is used.
  await page.keyboard.press('Control+1');
  await expect(stage).toHaveAttribute('data-level', '1');
  await emit(page, 'edt:tiles-progress', { key: 'tk1', level: 0 });
  await expect(stage).toHaveAttribute('data-level', '0');
  await expect
    .poll(async () => (await hook<{ shown: string[] }>(page, 'tiles')).shown.length)
    .toBeGreaterThan(1);
  // The backdrop stays loaded beneath (FR-EDT-08).
  const after = await hook<{ shown: string[]; backdropShown: boolean }>(page, 'tiles');
  expect(after.backdropShown).toBe(true);
  // Konva holds no view state of its own (FR-EDT-06).
  const v = await view(page);
  const stageAttrs = await page.evaluate(() => {
    const s = window.__pfTest!.stage!();
    return { x: s.x(), y: s.y(), sx: s.scaleX(), sy: s.scaleY() };
  });
  expect(stageAttrs).toEqual({ x: v.tx, y: v.ty, sx: v.scale, sy: v.scale });
});

test('FR-EDT-07 a pyramid built and reported before tiles_open returns still loads (D1)', async ({
  page,
}) => {
  // openMap waits for data-image-loaded; before the fix the events were missed and it stayed false.
  await openMap(page, { buildOnOpen: 'ready' });
  await expect(page.getByTestId('edt-loading')).toBeHidden();
  const t = await hook<{ backdropShown: boolean; level: number | null }>(page, 'tiles');
  expect(t.backdropShown).toBe(true);
  expect(t.level).not.toBeNull();
});

test('FR-EDT-07 a build that fails before tiles_open returns shows the error, not the spinner (D1)', async ({
  page,
}) => {
  await openMap(page, { buildOnOpen: 'failed' });
  await expect(page.getByTestId('edt-build-error')).toContainText('image body is unreadable');
  await expect(page.getByTestId('edt-loading')).toBeHidden();
  // Rebuild reopens; this time the pyramid is complete.
  await page.getByTestId('edt-build-error').getByRole('button', { name: 'Rebuild' }).click();
  await expect(page.getByTestId('edt-build-error')).toBeHidden();
  await expect(page.getByTestId('edt-canvas-stage')).toHaveAttribute('data-image-loaded', 'true');
});

test('FR-EDT-12 a failing tile shows the badge; Rebuild reopens the pyramid', async ({ page }) => {
  await openMap(page);
  // Registered after the mock's catch-all route, so it wins.
  await page.route('http://pf-tiles.localhost/t/tk1/0/**', (r) =>
    r.fulfill({ status: 404, headers: { 'Access-Control-Allow-Origin': '*' } }),
  );
  await page.keyboard.press('Control+1');
  await expect(page.getByTestId('edt-tiles-error')).toBeVisible({ timeout: 5000 });
  // The lower level stays visible.
  expect((await hook<{ backdropShown: boolean }>(page, 'tiles')).backdropShown).toBe(true);
  await page.getByTestId('edt-tiles-rebuild').click();
  await expect(page.getByTestId('edt-tiles-error')).toBeHidden();
  const opens = await page.evaluate(
    () => (window as unknown as { __pfEdtTilesOpen: { rebuild?: boolean }[] }).__pfEdtTilesOpen,
  );
  expect(opens.at(-1)?.rebuild).toBe(true);
});

test('FR-EDT-03 pan with the pan tool, middle-drag and Space+drag; 64 px stay visible', async ({
  page,
}) => {
  await openMap(page);
  const box = await stageBox(page);
  const c = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  const v0 = await view(page);

  // Pan tool (default, H).
  await page.keyboard.press('h');
  await page.mouse.move(c.x, c.y);
  await page.mouse.down();
  await page.mouse.move(c.x + 40, c.y + 20, { steps: 4 });
  await page.mouse.up();
  let v = await view(page);
  expect([v.tx - v0.tx, v.ty - v0.ty]).toEqual([40, 20]);

  // Middle-drag with another tool.
  await page.keyboard.press('r');
  await page.mouse.move(c.x, c.y);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(c.x - 30, c.y, { steps: 3 });
  await page.mouse.up({ button: 'middle' });
  v = await view(page);
  expect(v.tx - v0.tx).toBeCloseTo(10, 6);

  // Space + drag.
  await page.keyboard.down('Space');
  await page.mouse.move(c.x, c.y);
  await page.mouse.down();
  await page.mouse.move(c.x, c.y + 50, { steps: 3 });
  await page.mouse.up();
  await page.keyboard.up('Space');
  v = await view(page);
  expect(v.ty - v0.ty).toBeCloseTo(70, 6);

  // Far drag: at least 64 px of the image stay inside.
  await page.keyboard.press('h');
  await page.mouse.move(c.x, c.y);
  await page.mouse.down();
  await page.mouse.move(c.x + 5000, c.y + 5000, { steps: 5 });
  await page.mouse.up();
  v = await view(page);
  expect(v.tx).toBeCloseTo(box.width - 64, 3);
  expect(v.ty).toBeCloseTo(box.height - 64, 3);
});

test('FR-EDT-30 tool shortcuts are ignored while a text field has focus', async ({ page }) => {
  await openMap(page);
  const field = page.getByTestId('prj-panel-history-label');
  await field.click();
  await page.keyboard.press('b');
  await page.keyboard.press(']');
  await expect(page.getByTestId('edt-toolbar-pan')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('edt-toolbar-size')).toHaveAttribute('data-value', '64');
  await expect(field).toHaveValue('b]');
  await page.getByTestId('edt-canvas-stage').click();
  await page.keyboard.press('b');
  await page.keyboard.press(']');
  await expect(page.getByTestId('edt-toolbar-brush')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('edt-toolbar-size')).toHaveAttribute('data-value', '70');
  expect((await calls(page)).some((c) => c.cmd === 'edt_tiles_open')).toBe(true);
});

test('FR-EDT-03 R41 Space pans right after a mouse click on a tool or preset button', async ({
  page,
}) => {
  await openMap(page);
  const box = await stageBox(page);
  const c = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  const focused = () => page.evaluate(() => document.activeElement?.getAttribute('data-testid'));
  const spaceDrag = async (dy: number) => {
    const v0 = await view(page);
    await page.keyboard.down('Space');
    await page.mouse.move(c.x, c.y);
    await page.mouse.down();
    await page.mouse.move(c.x, c.y + dy, { steps: 3 });
    await page.mouse.up();
    await page.keyboard.up('Space');
    const v = await view(page);
    return v.ty - v0.ty;
  };

  // Click the brush tool: the tool changes, the button doesn't keep focus, Space+drag pans.
  await page.getByTestId('edt-toolbar-brush').click();
  await expect(page.getByTestId('edt-toolbar-brush')).toHaveAttribute('aria-pressed', 'true');
  expect(await focused()).not.toBe('edt-toolbar-brush');
  expect(await spaceDrag(40)).toBeCloseTo(40, 6);
  await expect(page.getByTestId('edt-toolbar-brush')).toHaveAttribute('aria-pressed', 'true');

  // The same after a view preset.
  await page.getByTestId('edt-view-fit').click();
  expect(await focused()).not.toBe('edt-view-fit');
  expect(await spaceDrag(30)).toBeCloseTo(30, 6);

  // Keyboard focus is unchanged: a focused button still gets Space first (#48).
  await page.getByTestId('edt-view-fit').focus();
  expect(await focused()).toBe('edt-view-fit');
});

test('FR-EDT-03 FR-EDT-24 a focus loss releases a held Space and backslash', async ({ page }) => {
  await openMap(page);
  const stage = page.getByTestId('edt-canvas-stage');
  const flip = async () => (await hook<{ flipBefore: boolean }>(page, 'store')).flipBefore;
  await page.keyboard.press('r');
  await page.keyboard.down('Space');
  await page.keyboard.down('Backslash');
  await expect(stage).toHaveClass(/editor-canvas-pan/);
  expect(await flip()).toBe(true);
  // Alt-Tab: the keyups go to another window.
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await expect(stage).not.toHaveClass(/editor-canvas-pan/);
  expect(await flip()).toBe(false);
});
