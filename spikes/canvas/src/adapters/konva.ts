import Konva from 'konva';
import type { Adapter, PatchProps } from './types';
import type { Pt, View } from '../core/view';
import type { Surface } from '../core/mask';

export class KonvaAdapter implements Adapter {
  readonly name = 'konva' as const;
  private stage!: Konva.Stage;
  private tileLayer!: Konva.Layer;
  private base!: Konva.Group;
  private top!: Konva.Group;
  private overlay!: Konva.Layer;
  private tiles = new Map<string, Konva.Image>();
  private masks = new Map<Surface, Konva.Image>();
  private patch?: Konva.Image;
  private tr?: Konva.Transformer;

  async init(el: HTMLElement, w: number, h: number) {
    Konva.pixelRatio = 1;
    this.stage = new Konva.Stage({ container: el as HTMLDivElement, width: w, height: h });
    this.tileLayer = new Konva.Layer({ listening: false });
    this.base = new Konva.Group(); this.top = new Konva.Group();
    this.tileLayer.add(this.base, this.top);
    this.overlay = new Konva.Layer();
    this.stage.add(this.tileLayer, this.overlay);
  }

  setView(v: View) {
    this.stage.scale({ x: v.scale, y: v.scale });
    this.stage.position({ x: v.tx, y: v.ty });
  }

  render() { this.tileLayer.draw(); this.overlay.draw(); }

  screenToImage(p: Pt): Pt { return this.stage.getAbsoluteTransform().copy().invert().point(p); }
  imageToScreen(p: Pt): Pt { return this.stage.getAbsoluteTransform().point(p); }

  addTile(key: string, bmp: ImageBitmap, x: number, y: number, w: number, h: number, z: 0 | 1) {
    const n = new Konva.Image({ image: bmp, x, y, width: w, height: h, listening: false, perfectDrawEnabled: false });
    (z ? this.top : this.base).add(n);
    this.tiles.set(key, n);
  }
  removeTile(key: string) { this.tiles.get(key)?.destroy(); this.tiles.delete(key); }

  addMask(surfaces: Surface[], opacity: number) {
    for (const s of surfaces) {
      const n = new Konva.Image({ image: s.canvas, x: s.x, y: s.y, width: s.w, height: s.h, opacity, listening: false });
      this.overlay.add(n); this.masks.set(s, n);
    }
  }
  updateMask(_dirty: Surface[]) { /* drawImage(canvas) reads the live canvas; nothing to upload */ }

  addPatch(img: CanvasImageSource & { width: number; height: number }, p: PatchProps) {
    this.patch = new Konva.Image({ image: img, width: img.width, height: img.height, offsetX: img.width / 2, offsetY: img.height / 2 });
    this.overlay.add(this.patch);
    this.setPatch(p);
  }
  setPatch(p: Partial<PatchProps>) {
    const n = this.patch!;
    if (p.cx !== undefined) n.x(p.cx);
    if (p.cy !== undefined) n.y(p.cy);
    if (p.scale !== undefined) n.scale({ x: p.scale, y: p.scale });
    if (p.rotation !== undefined) n.rotation(p.rotation);
    if (p.opacity !== undefined) n.opacity(p.opacity);
    if (p.visible !== undefined) n.visible(p.visible);
  }
  patchCorners(): Pt[] {
    const n = this.patch!, t = n.getTransform(), w = n.width(), h = n.height();
    return [{ x: 0, y: 0 }, { x: w, y: 0 }, { x: w, y: h }, { x: 0, y: h }].map(p => t.point(p));
  }
  enableHandles(on: boolean) {
    const n = this.patch!;
    n.draggable(on);
    if (on && !this.tr) { this.tr = new Konva.Transformer({ nodes: [n], keepRatio: true, rotateEnabled: true }); this.overlay.add(this.tr); }
    if (!on) { this.tr?.destroy(); this.tr = undefined; }
    return { move: true, scale: true, rotate: true, builtIn: true };
  }
  setInteractive(on: boolean) { this.stage.listening(on); }
  info() { return { version: Konva.version, nodes: this.tiles.size, canvases: 2 }; }
  destroy() { this.stage.destroy(); }
}
