// FR-EDT-26: `window.__pfTest`, in dev and test builds only. This module is loaded through a
// dynamic import behind a compile-time flag (see `hook.ts`), so production bundles don't contain
// it; `scripts/check-no-test-hook.mjs` greps the build to prove it.
import Konva from 'konva';

import type { TestHookSources } from './hook';

export function install(src: TestHookSources): () => void {
  /** A node's attributes and its corners in image px (stage transform removed, FR-EDT-13). */
  const nodeInfo = (n: Konva.Node) => {
    const stage = n.getStage();
    const m = stage ? stage.getAbsoluteTransform().copy().invert() : new Konva.Transform();
    m.multiply(n.getAbsoluteTransform().copy());
    const w = n.width();
    const h = n.height();
    const { image: _image, ...attrs } = n.getAttrs() as Record<string, unknown>;
    void _image;
    return {
      name: n.name(),
      id: n.id(),
      className: n.getClassName(),
      attrs,
      visible: n.isVisible(),
      corners: [
        m.point({ x: 0, y: 0 }),
        m.point({ x: w, y: 0 }),
        m.point({ x: w, y: h }),
        m.point({ x: 0, y: h }),
      ],
    };
  };
  const base: NonNullable<Window['__pfTest']> = {
    view: () => src.store().view,
    dpr: () => src.store().dpr,
    pixelRatio: () => src.stage()?.getLayers()[0]?.getCanvas().getPixelRatio() ?? null,
    imageSize: () => src.store().image,
    imageLoaded: () => src.tiles()?.backdropShown ?? false,
    tiles: () => src.tiles(),
    stage: () => src.stage(),
    find: (selector: string) => (src.stage()?.find(selector) ?? []).map(nodeInfo),
    store: () => src.store(),
  };
  // Later slices add entries through `extra()`, read on every access.
  window.__pfTest = new Proxy(base, {
    get: (t, k: string) => t[k] ?? src.extra()[k],
  });
  return () => {
    delete window.__pfTest;
  };
}
