import { test, expect } from '@playwright/test';
import { launch, save, LIBS } from './helpers';

// Patch layer: transform read back from the library must match the image-px affine Rust will composite (KQ-25).
for (const lib of LIBS) {
  test(`patch layer ${lib}`, async () => {
    const { browser, page } = await launch({ vsync: true });
    await page.evaluate(l => window.bench.setup(l), lib);
    await page.evaluate(() => window.bench.loadTiles(8192));
    const r = await page.evaluate(() => window.bench.layers());
    await page.evaluate(async () => { window.bench.a!.render(); await new Promise(r => requestAnimationFrame(r)); });
    await page.locator('#stage').screenshot({ path: `test-results/layers/${lib}.png` });
    save(`layers-${lib}`, r);
    console.log(lib, JSON.stringify(r));
    expect(r.cornerErrPx).toBeLessThan(1e-6);
    await browser.close();
  });
}
