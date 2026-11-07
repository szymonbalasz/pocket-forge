// FR-EDT-24: draws the compare "after" image over its rect in image px, clipped to the visible
// part (split: right of the divider; flip: all of it, or nothing while the before key is held).
import Konva from 'konva';

import { afterClip } from './compare';
import type { CompareMode, CompareSource } from './store';

export class CompareLayer {
  private readonly group: Konva.Group;
  private node: Konva.Image | null = null;
  private url: string | null = null;
  private gen = 0;

  constructor(
    private readonly layer: Konva.Layer,
    private readonly onError: (msg: string | null) => void,
  ) {
    this.group = new Konva.Group({ name: 'compare', listening: false });
    layer.add(this.group);
  }

  update(src: CompareSource | null, mode: CompareMode, split: number, before: boolean): void {
    if (!src) {
      this.clear();
      return;
    }
    const clip = afterClip(src.rect, mode, split, before);
    this.group.clip({ x: clip.x, y: clip.y, width: clip.w, height: clip.h });
    if (src.url !== this.url) {
      this.clear();
      this.url = src.url;
      const gen = ++this.gen;
      void (async () => {
        try {
          const res = await fetch(src.url);
          if (!res.ok) throw new Error(`HTTP ${res.status}`);
          const bitmap = await createImageBitmap(await res.blob());
          if (gen !== this.gen) return bitmap.close();
          this.node = new Konva.Image({
            image: bitmap,
            name: 'compare-after',
            x: src.rect.x,
            y: src.rect.y,
            width: src.rect.w,
            height: src.rect.h,
            listening: false,
          });
          this.group.add(this.node);
          this.onError(null);
          this.layer.batchDraw();
        } catch (e) {
          if (gen === this.gen) this.onError((e as Error).message);
        }
      })();
    } else if (this.node) {
      this.node.position({ x: src.rect.x, y: src.rect.y });
      this.node.size({ width: src.rect.w, height: src.rect.h });
    }
    this.layer.batchDraw();
  }

  /** The current clip in image px (test hook). */
  clip(): { x: number; y: number; width: number; height: number } | null {
    return this.url
      ? (this.group.clip() as { x: number; y: number; width: number; height: number })
      : null;
  }

  loaded(): boolean {
    return !!this.node;
  }

  private clear(): void {
    this.gen++;
    this.url = null;
    this.node?.destroy();
    this.node = null;
    this.layer.batchDraw();
  }

  destroy(): void {
    this.gen++;
    this.group.destroy();
  }
}
