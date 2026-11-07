import { test } from '@playwright/test';
import { launch, LIBS } from './helpers';

// Visual smoke: each library, tiles + single, screenshots at fit and at a fractional zoom (seam check).
for (const lib of LIBS) {
  test(`smoke ${lib}`, async () => {
    const { browser, page } = await launch({ vsync: true });
    const stage = page.locator('#stage');
    await page.evaluate(l => window.bench.setup(l), lib);
    console.log(lib, 'tiles', JSON.stringify(await page.evaluate(() => window.bench.loadTiles(8192))));
    await stage.screenshot({ path: `test-results/smoke/${lib}-tiles-fit.png` });
    await page.evaluate(async () => {
      const b = window.bench;
      for (let i = 0; i < 200; i++) { b.apply({ scale: 0.7371, tx: -2003.37, ty: -1501.61 }); b.a!.render(); await new Promise(r => requestAnimationFrame(r)); }
    });
    await stage.screenshot({ path: `test-results/smoke/${lib}-tiles-0.737.png` });
    await page.evaluate(l => window.bench.setup(l), lib);
    console.log(lib, 'single', JSON.stringify(await page.evaluate(() => window.bench.loadSingle(8192))));
    await stage.screenshot({ path: `test-results/smoke/${lib}-single-fit.png` });
    await browser.close();
  });
}
