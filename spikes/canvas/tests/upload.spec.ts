import { test } from '@playwright/test';
import { launch, LIBS } from './helpers';

// Diagnostic: cost of first-showing 32 new 512² tiles, per library and ImageBitmap option.
const libs = (process.env.LIBS?.split(',') ?? LIBS) as typeof LIBS[number][];
for (const lib of libs) test(`upload ${lib}`, async () => {
  const { browser, page } = await launch({ highPerfGpu: process.env.GPU === 'dgpu' });
  await page.evaluate(l => window.bench.setup(l), lib);
  let off = 0;
  for (const kind of ['default', 'default', 'premul', 'none', 'nocs', 'img', 'default'] as const) {
    console.log(lib, JSON.stringify(await page.evaluate(([k, o]) => window.bench.uploadProbe(32, k as 'default', o as number), [kind, off] as const)));
    off += 32;
  }
  await browser.close();
});
