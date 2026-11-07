import { test, expect } from '@playwright/test';
import { centerOn, fit, imageToScreen, pixelAt, screenToImage, zoomAbout, type View } from '../src/core/view';

// Pure math (runs in Node): the shared view transform has zero drift.
test('screen ↔ image round trip is exact to 1e-9 px at all zooms', () => {
  for (const z of [1 / 128, 1 / 64, 1 / 7, 0.3719, 1, 2.5, 8, 32, 64]) {
    const v = centerOn(z, 16383.7, 9.1, 800.5, 450.25);
    for (let i = 0; i < 1000; i++) {
      const p = { x: (i * 7919) % 16384 + 0.3, y: (i * 104729) % 16384 + 0.7 };
      const q = screenToImage(v, imageToScreen(v, p));
      expect(Math.hypot(q.x - p.x, q.y - p.y)).toBeLessThan(1e-9);
    }
  }
});

test('pixelAt picks the pixel whose square contains the point', () => {
  const v = centerOn(8, 100, 100, 0, 0);
  expect(pixelAt(v, imageToScreen(v, { x: 42.999, y: 7.001 }))).toEqual({ x: 42, y: 7 });
  expect(pixelAt(v, imageToScreen(v, { x: 43, y: 7 }))).toEqual({ x: 43, y: 7 });
});

test('anchored zoom keeps the point under the cursor fixed over 10k steps', () => {
  let v: View = fit({ x: 0, y: 0, w: 16384, h: 16384 }, 1600, 900);
  let worst = 0;
  for (let i = 0; i < 10000; i++) {
    const sx = (i * 37) % 1600, sy = (i * 91) % 900, before = screenToImage(v, { x: sx, y: sy });
    v = zoomAbout(v, i % 3 ? 1.1 : 1 / 1.1, sx, sy, 0, Infinity);
    const after = screenToImage(v, { x: sx, y: sy });
    worst = Math.max(worst, Math.hypot(after.x - before.x, after.y - before.y));
  }
  expect(worst).toBeLessThan(1e-6);
});
