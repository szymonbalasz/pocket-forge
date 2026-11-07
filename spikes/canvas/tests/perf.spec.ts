import { test } from '@playwright/test';
import { launch, procMem, save, LIBS } from './helpers';

// Large image display: time to first view, scripted 9 s pan/zoom frame times, JS heap, GPU + process memory.
// GPU=dgpu runs with --force_high_performance_gpu (hybrid laptops default to the iGPU).
const gpu = process.env.GPU ?? 'igpu';
const sizes = (process.env.SIZES ?? '8192,16384').split(',').map(Number);
const modes = (process.env.MODES ?? 'tiles,single').split(',') as ('tiles' | 'single')[];
const libs = (process.env.LIBS?.split(',') ?? LIBS) as typeof LIBS[number][];

for (const lib of libs) for (const mode of modes) for (const size of sizes) {
  test(`perf ${lib} ${mode} ${size} ${gpu}`, async () => {
    const { browser, page } = await launch({ highPerfGpu: gpu === 'dgpu', vsync: !process.env.UNCAPPED });
    await page.evaluate(l => window.bench.setup(l), lib);
    const memIdle = procMem();
    const load = mode === 'tiles' ? await page.evaluate(s => window.bench.loadTiles(s), size) : await page.evaluate(s => window.bench.loadSingle(s), size);
    await page.waitForTimeout(1000);
    const memLoaded: Record<string, number> = { ...procMem(), ...(await page.evaluate(() => window.bench.mem())) };
    const pz = 'ok' in load && load.ok === false ? null : await page.evaluate(() => window.bench.panZoom());
    const memAfter: Record<string, number> = { ...procMem(), ...(await page.evaluate(() => window.bench.mem())) };
    const r = { lib, mode, size, gpu, load, panZoom: pz, memIdle, memLoaded, memAfter };
    save(`perf-${lib}-${mode}-${size}-${gpu}`, r);
    console.log(JSON.stringify({ lib, mode, size, load, fps: pz?.fps, p95: pz?.frameP95, p99: pz?.frameP99, max: pz?.frameMax, cpuP95: pz?.cpuP95, over33: pz?.over33ms, gpuMB: memAfter.gpuDedicatedMB, gpuShMB: memAfter.gpuSharedMB, privMB: memAfter.privateMB, heap: memAfter.jsHeapUsedMB }));
    await browser.close();
  });
}
