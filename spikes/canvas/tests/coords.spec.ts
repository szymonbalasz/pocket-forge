import { test, expect } from '@playwright/test';
import sharp from 'sharp';
import { launch, save, LIBS } from './helpers';

// Each library's own screen↔image conversion, plus rendered alignment: a 64² probe with a unique colour
// per pixel at image (16000,16000) must land exactly where the shared math says, at zoom 8 and 32.
for (const lib of LIBS) {
  test(`coordinates ${lib}`, async () => {
    const { browser, page } = await launch({ vsync: true });
    await page.evaluate(l => window.bench.setup(l), lib);
    const c = await page.evaluate(() => window.bench.coords());
    const rendered: Record<string, unknown> = {};
    for (const zoom of [8, 32]) {
      const samples = await page.evaluate(z => window.bench.probe(z), zoom);
      const png = await page.locator('#stage').screenshot({ path: `test-results/coords/${lib}-z${zoom}.png` });
      const { data, info } = await sharp(png).raw().toBuffer({ resolveWithObject: true });
      let bad = 0, worst = 0;
      for (const s of samples) {
        const o = (s.sy * info.width + s.sx) * info.channels;
        const d = Math.max(Math.abs(data[o] - s.r), Math.abs(data[o + 1] - s.g), Math.abs(data[o + 2] - s.b));
        worst = Math.max(worst, d);
        // a 1-px misplacement flips blue by 100 and shifts r/g by 4; bilinear blending near a centre stays well under these
        if (Math.abs(data[o + 2] - s.b) > 40 || Math.abs(data[o] - s.r) > 3 || Math.abs(data[o + 1] - s.g) > 3) bad++;
      }
      rendered[`zoom${zoom}`] = { samples: samples.length, mismatched: bad, worstChannelDiff: worst };
    }
    save(`coords-${lib}`, { ...c, rendered });
    console.log(lib, JSON.stringify({ worstRoundTrip: Math.max(...c.rows.map(r => r.roundTripMaxPx)), worstVsCore: Math.max(...c.rows.map(r => r.vsCoreMaxScreenPx)), pixelMismatch: c.rows.reduce((a, r) => a + r.pixelMismatch, 0), anchorDrift: c.anchorDriftMaxImagePx, returnTrip: c.returnTrip, rendered }));
    for (const r of c.rows) { expect(r.roundTripMaxPx).toBeLessThan(1e-6); expect(r.vsCoreMaxScreenPx).toBeLessThan(1e-6); expect(r.pixelMismatch).toBe(0); }
    expect(c.anchorDriftMaxImagePx).toBeLessThan(1e-6);
    for (const r of Object.values(rendered) as { mismatched: number }[]) expect(r.mismatched).toBe(0);
    await browser.close();
  });
}
