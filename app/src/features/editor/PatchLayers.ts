// FR-EDT-13: layer display. Patch layers are drawn above the original from the current manifest
// layer stack, in order, each as a Konva.Image with its centre at (cx, cy), offset by half its
// size, scaled and rotated about the centre (ADR 0003). Display only: no Transformer (tier A).
// Imperative (OQ-CNV-06): the stack is diffed by layer id, images loaded through edt_image_open.
import Konva from 'konva';

import { api, type Layer } from '../../api';

export interface LoadedLayer {
  w: number;
  h: number;
  image: CanvasImageSource;
}

export type LoadLayer = (projectId: string, layerId: string) => Promise<LoadedLayer>;

/** Loads a layer file through `edt_image_open` (≤ 4096 px; layer files are write-once, R3, so the
 *  key is stable and the bitmap can be reused for the session). */
export const loadLayer: LoadLayer = async (projectId, layerId) => {
  const img = await api.editor.imageOpen(projectId, layerId);
  const res = await fetch(img.url);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const bitmap = await createImageBitmap(await res.blob());
  return { w: img.width, h: img.height, image: bitmap };
};

/** The affine (ADR 0003): image px of a layer-local point (u, v) ∈ [0, w] × [0, h]. */
export function layerPoint(l: Layer, w: number, h: number, u: number, v: number) {
  const r = (l.rotation * Math.PI) / 180;
  const dx = (u - w / 2) * l.scale;
  const dy = (v - h / 2) * l.scale;
  return {
    x: l.cx + dx * Math.cos(r) - dy * Math.sin(r),
    y: l.cy + dx * Math.sin(r) + dy * Math.cos(r),
  };
}

export class PatchLayers {
  private readonly loaded = new Map<string, LoadedLayer | 'failed' | 'loading'>();
  private stack: Layer[] = [];
  private destroyed = false;

  constructor(
    private readonly layer: Konva.Layer,
    private readonly projectId: string,
    private readonly onError: (layerId: string, message: string) => void,
    private readonly load: LoadLayer = loadLayer,
  ) {}

  /** Replace the drawn stack with `layers` (the manifest's, original first). */
  setStack(layers: readonly Layer[]): void {
    this.stack = layers.filter((l) => l.kind === 'patch');
    for (const l of this.stack) {
      if (!this.loaded.has(l.id)) {
        this.loaded.set(l.id, 'loading');
        this.load(this.projectId, l.id).then(
          (img) => {
            if (this.destroyed) return;
            this.loaded.set(l.id, img);
            this.render();
          },
          (e: Error) => {
            if (this.destroyed) return;
            this.loaded.set(l.id, 'failed');
            this.onError(l.id, e.message);
          },
        );
      }
    }
    this.render();
  }

  /** Ids drawn, in order (hidden and still-loading layers are absent). */
  drawn(): string[] {
    return this.layer.getChildren().map((n) => n.id());
  }

  destroy(): void {
    this.destroyed = true;
    this.layer.destroyChildren();
  }

  private render(): void {
    if (this.destroyed) return;
    this.layer.destroyChildren();
    for (const l of this.stack) {
      const img = this.loaded.get(l.id);
      if (!l.visible || !img || typeof img === 'string') continue;
      this.layer.add(
        new Konva.Image({
          id: l.id,
          name: 'patch-layer',
          image: img.image,
          x: l.cx,
          y: l.cy,
          width: img.w,
          height: img.h,
          offsetX: img.w / 2,
          offsetY: img.h / 2,
          scaleX: l.scale,
          scaleY: l.scale,
          rotation: l.rotation,
          opacity: l.opacity,
          listening: false,
        }),
      );
    }
    this.layer.batchDraw();
  }
}
