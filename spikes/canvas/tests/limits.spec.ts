import { test, expect } from '@playwright/test';
import { launch, save, LIBS } from './helpers';

test('engine canvas/WebGL limits and 16k decode', async () => {
  const { browser, page } = await launch();
  const limits = await page.evaluate(() => window.bench.limits());
  const webgpu = await page.evaluate(() => window.bench.webgpuLimits());
  const perLib: Record<string, unknown> = {};
  for (const lib of LIBS) perLib[lib] = await page.evaluate(l => window.bench.setup(l), lib);
  save('limits', { limits, webgpu, perLib, browserVersion: browser.version() });
  console.log(JSON.stringify({ limits, webgpu, perLib }, null, 1));
  expect(limits.webgl2.maxTextureSize).toBeGreaterThan(0);
  await browser.close();
});
