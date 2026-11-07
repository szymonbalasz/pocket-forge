import { describe, expect, test } from 'vitest';

import {
  fit,
  imageToScreen,
  panBy,
  scaleLimits,
  screenToImage,
  wheelFactor,
  zoomAbout,
} from './view';

const image = { w: 2100, h: 2800 };
const viewport = { w: 1200, h: 800 };

describe('view math', () => {
  test('FR-EDT-01 fit centres the whole image inside the viewport', () => {
    const v = fit(image, viewport);
    const tl = imageToScreen(v, { x: 0, y: 0 });
    const br = imageToScreen(v, { x: image.w, y: image.h });
    expect(tl.x).toBeGreaterThanOrEqual(0);
    expect(br.x).toBeLessThanOrEqual(viewport.w + 1e-9);
    expect(tl.y).toBeCloseTo(0);
    expect(br.y).toBeCloseTo(viewport.h);
    expect((tl.x + br.x) / 2).toBeCloseTo(viewport.w / 2);
  });

  test('FR-EDT-02 one notch is 2^¼ and the anchor drifts ≤ 0.5 px after 2000 notches', () => {
    expect(wheelFactor(100)).toBeCloseTo(2 ** -0.25);
    expect(wheelFactor(-100)).toBeCloseTo(2 ** 0.25);
    const anchor = { x: 437.25, y: 291.5 };
    let v = fit(image, viewport);
    const target = screenToImage(v, anchor);
    const { min, max } = scaleLimits(image, viewport, 1);
    for (let i = 0; i < 2000; i++)
      v = zoomAbout(v, wheelFactor(i % 2 ? 100 : -100), anchor, min, max);
    const p = imageToScreen(v, target);
    expect(Math.hypot(p.x - anchor.x, p.y - anchor.y)).toBeLessThanOrEqual(0.5);
  });

  test('FR-EDT-02 scale is clamped to [0.5 × fit, 32 device px]', () => {
    const { min, max } = scaleLimits(image, viewport, 2);
    let v = fit(image, viewport);
    for (let i = 0; i < 100; i++) v = zoomAbout(v, 2, { x: 0, y: 0 }, min, max);
    expect(v.scale).toBe(16);
    for (let i = 0; i < 100; i++) v = zoomAbout(v, 0.5, { x: 0, y: 0 }, min, max);
    expect(v.scale).toBeCloseTo(fit(image, viewport).scale / 2);
  });

  test('FR-EDT-03 panning keeps at least 64 px of the image visible', () => {
    const v = panBy(fit(image, viewport), 1e6, -1e6, image, viewport);
    expect(v.tx).toBe(viewport.w - 64);
    expect(v.ty + image.h * v.scale).toBe(64);
  });
});
