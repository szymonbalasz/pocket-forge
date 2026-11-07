// EDT slice 3: layer display from the manifest stack (FR-EDT-13).
import { expect, test } from '@playwright/test';

import { hook, MAP, openMap } from './edt-mock';
import { emit } from './mock-core';

const original = {
  id: '01JLAYER00000000000000000',
  kind: 'original',
  file: null,
  cx: MAP.w / 2,
  cy: MAP.h / 2,
  scale: 1,
  rotation: 0,
  opacity: 1,
  visible: true,
};
const patch = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  kind: 'patch',
  file: `layers/${id}.png`,
  cx: 600,
  cy: 400,
  scale: 1.5,
  rotation: 30,
  opacity: 0.9,
  visible: true,
  ...over,
});

test('FR-EDT-13 patch layers follow the manifest stack, hidden ones are not drawn, and refresh on commit', async ({
  page,
}) => {
  await page.addInitScript(
    ({ layers }) => {
      const w = window as unknown as Record<string, unknown>;
      w.__pfMockSnapshot = { layers };
      const h = (w.__pfMockHandlers ??= {}) as Record<string, (i: unknown) => unknown>;
      h.edt_image_open = () => ({
        key: 'lk',
        url: 'http://pf-tiles.localhost/i/lk.png',
        width: 200,
        height: 100,
      });
    },
    {
      layers: [
        original,
        patch('PA'),
        patch('PB', { visible: false }),
        patch('PC', { cx: 900, rotation: 0, scale: 1 }),
      ],
    },
  );
  await openMap(page);
  await expect.poll(() => hook<string[]>(page, 'layers')).toEqual(['PA', 'PC']);

  // Read-back corners equal the affine about the centre (ADR 0003).
  const [a] = await hook<{ id: string; corners: { x: number; y: number }[] }[]>(
    page,
    'find',
    '#PA',
  );
  const r = (30 * Math.PI) / 180;
  const corner = (u: number, v: number) => {
    const dx = (u - 100) * 1.5;
    const dy = (v - 50) * 1.5;
    return {
      x: 600 + dx * Math.cos(r) - dy * Math.sin(r),
      y: 400 + dx * Math.sin(r) + dy * Math.cos(r),
    };
  };
  const want = [corner(0, 0), corner(200, 0), corner(200, 100), corner(0, 100)];
  a!.corners.forEach((c, i) => {
    expect(Math.abs(c.x - want[i]!.x)).toBeLessThan(0.01);
    expect(Math.abs(c.y - want[i]!.y)).toBeLessThan(0.01);
  });

  // A commit (e.g. accept, re-tune) changes the stack: the drawn stack follows.
  await page.evaluate(
    ({ layers }) => {
      (window as unknown as Record<string, unknown>).__pfMockSnapshot = { layers };
    },
    { layers: [original, patch('PC', { cx: 900, rotation: 0, scale: 1 }), patch('PD')] },
  );
  await emit(page, 'project:committed', { seq: 2 });
  await expect.poll(() => hook<string[]>(page, 'layers')).toEqual(['PC', 'PD']);
});
