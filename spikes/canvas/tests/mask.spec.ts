import { test } from '@playwright/test';
import { launch, save, LIBS } from './helpers';

// Mask painting at full resolution over a region of the 16k map (tiles underneath), 300 frames of
// scripted zig-zag strokes (one segment ≈ 0.6·r per frame; every third stroke erases), r = 60 px, feather 50 %.
const gpu = process.env.GPU ?? 'igpu';
const libs = (process.env.LIBS?.split(',') ?? LIBS) as typeof LIBS[number][];
for (const lib of libs) for (const side of [2048, 4096]) for (const storage of ['region', 'tiles'] as const) {
  test(`mask ${lib} ${side} ${storage} ${gpu}`, async () => {
    const { browser, page } = await launch({ highPerfGpu: gpu === 'dgpu', vsync: !process.env.UNCAPPED });
    await page.evaluate(l => window.bench.setup(l), lib);
    await page.evaluate(() => window.bench.loadTiles(16384));
    const r = await page.evaluate(o => window.bench.maskPaint(o), { region: { x: 6000, y: 6000, w: side, h: side }, storage, radius: 60, hardness: 0.5, frames: 300 });
    await page.locator('#stage').screenshot({ path: `test-results/mask/${lib}-${side}-${storage}.png` });
    save(`mask-${lib}-${side}-${storage}-${gpu}`, { lib, side, storage, gpu, ...r });
    console.log(JSON.stringify({ lib, side, storage, fps: r.fps, p50: r.frameP50, p95: r.frameP95, max: r.frameMax, cpuP50: r.cpuP50, cpuP95: r.cpuP95, dabP95: r.dabP95, exportMs: r.exportMs, painted: r.paintedFraction }));
    await browser.close();
  });
}
