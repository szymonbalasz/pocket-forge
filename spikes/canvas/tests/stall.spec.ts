import { test } from '@playwright/test';
import { launch, LIBS } from './helpers';

// Diagnostic: where do long frames happen during pan/zoom (tiles, 16k)? Two passes in the same page:
// the second pass reuses compiled shaders and cached tiles.
const libs = (process.env.LIBS?.split(',') ?? LIBS) as typeof LIBS[number][];
for (const lib of libs) test(`stall ${lib}`, async () => {
  const { browser, page } = await launch({ highPerfGpu: process.env.GPU === 'dgpu', vsync: !!process.env.VSYNC });
  if (process.env.PIXI_KEEP) await page.evaluate(() => { (window as unknown as { __pixiKeep: boolean }).__pixiKeep = true; });
  await page.evaluate(l => window.bench.setup(l), lib);
  console.log(lib, 'renderer', (await page.evaluate(() => window.bench.limits())).webgl2.renderer);
  console.log(lib, 'load', JSON.stringify(await page.evaluate(() => window.bench.loadTiles(16384))));
  for (const pass of [1, 2]) {
    const r = await page.evaluate(() => window.bench.panZoom());
    console.log(lib, 'pass', pass, JSON.stringify({ fps: r.fps, p95: r.frameP95, p99: r.frameP99, max: r.frameMax, loads: r.tileLoads, long: r.long }));
  }
  await browser.close();
});
