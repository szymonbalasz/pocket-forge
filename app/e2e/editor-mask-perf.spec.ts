// EDT §12 mask timings at the largest mask (BL-09): stroke frame times with a 200 px brush, and
// the frame an undo step lands in, over a 3840² region. Indicative only: headless timings vary
// with the machine, so this is a measurement, not a gate. Run it on purpose:
//   PF_PERF=1 corepack pnpm e2e editor-mask-perf      (add --headed for real GPU compositing)
import { expect, test, type Page } from '@playwright/test';

import { hook, openMap, stageBox, view } from './edt-mock';
import { defaultSeed } from './mock-core';

test.skip(!process.env.PF_PERF, 'timing run: set PF_PERF=1');

const SIDE = 3840;

async function at(page: Page, x: number, y: number) {
  const b = await stageBox(page);
  const v = await view(page);
  return { x: b.x + v.tx + x * v.scale, y: b.y + v.ty + y * v.scale };
}

/** Starts recording rAF intervals in the page. */
const startFrames = (page: Page) =>
  page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __framesOn: boolean };
    w.__frames = [];
    w.__framesOn = true;
    let last = performance.now();
    const tick = (t: number) => {
      w.__frames.push(t - last);
      last = t;
      if (w.__framesOn) requestAnimationFrame(tick);
    };
    requestAnimationFrame((t) => {
      last = t;
      requestAnimationFrame(tick);
    });
  });

const stopFrames = (page: Page) =>
  page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __framesOn: boolean };
    w.__framesOn = false;
    return w.__frames;
  });

const pct = (xs: number[], p: number) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))] ?? NaN;
};

test('EDT §12 mask stroke and undo timings at a 3840² mask (indicative)', async ({ page }) => {
  test.setTimeout(120_000);
  await openMap(page, { seed: { ...defaultSeed, image: { w: 4200, h: 4200 } } });
  // Whole region on screen: 0.15 image px per CSS px.
  await hook(page, 'setView', { scale: 0.15, tx: 10, ty: 10 });
  await page.keyboard.press('r');
  const a = await at(page, 100, 100);
  const b = await at(page, 100 + SIDE, 100 + SIDE);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(b.x, b.y, { steps: 4 });
  await page.mouse.up();
  await expect
    .poll(async () => (await hook<{ inner: { w: number } } | null>(page, 'region'))?.inner.w)
    .toBe(SIDE);

  await page.keyboard.press('b');
  const size = page.getByTestId('edt-toolbar-size');
  while (Number(await size.getAttribute('data-value')) < 200) await page.keyboard.press(']');

  // A zig-zag stroke across the region, one pointer event per frame-ish.
  const strokes: number[][] = [];
  for (let run = 0; run < 3; run++) {
    await startFrames(page);
    const p0 = await at(page, 300, 300 + run * 1000);
    await page.mouse.move(p0.x, p0.y);
    await page.mouse.down();
    for (let i = 1; i <= 120; i++) {
      const x = 300 + (i * 3200) / 120;
      const y = 300 + run * 1000 + (i % 2 ? 400 : 0);
      const p = await at(page, x, y);
      await page.mouse.move(p.x, p.y);
    }
    await page.mouse.up();
    await page.waitForTimeout(100);
    strokes.push((await stopFrames(page)).slice(1));
  }

  // Undo the strokes one at a time: the worst frame around each step.
  const undos: number[] = [];
  for (let i = 0; i < 3; i++) {
    const v0 = await hook<number>(page, 'maskVersion');
    await startFrames(page);
    await page.waitForTimeout(100);
    await page.keyboard.press('Control+z');
    await expect.poll(() => hook<number>(page, 'maskVersion')).toBeGreaterThan(v0);
    await page.waitForTimeout(150);
    undos.push(Math.max(...(await stopFrames(page))));
  }

  // Fill region: the worst frame (a full-mask update).
  await startFrames(page);
  await page.getByTestId('edt-toolbar-fill').click();
  await page.waitForTimeout(200);
  const fill = Math.max(...(await stopFrames(page)));

  const all = strokes.flat();
  const r = (n: number) => n.toFixed(1);
  console.log(
    `mask ${SIDE}²: stroke frames n=${all.length} p50 ${r(pct(all, 50))} p95 ${r(pct(all, 95))} ` +
      `max ${r(Math.max(...all))} ms · undo worst frame ${undos.map(r).join(' / ')} ms · ` +
      `fill worst frame ${r(fill)} ms`,
  );
});
