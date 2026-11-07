import { Canvas, FabricImage, Point, util, version } from 'fabric';
import type { Adapter, PatchProps } from './types';
import type { Pt, View } from '../core/view';
import type { Surface } from '../core/mask';

const STATIC = { originX: 'left', originY: 'top', objectCaching: false, selectable: false, evented: false } as const;

export class FabricAdapter implements Adapter {
  readonly name = 'fabric' as const;
  private c!: Canvas;
  private tiles = new Map<string, FabricImage>();
  private nBase = 0;
  private nTop = 0;
  private masks = new Map<Surface, FabricImage>();
  private patch?: FabricImage;

  async init(el: HTMLElement, w: number, h: number) {
    const cv = document.createElement('canvas');
    el.append(cv);
    this.c = new Canvas(cv, {
      width: w, height: h, renderOnAddRemove: false, skipOffscreen: true, selection: false,
      enableRetinaScaling: false, preserveObjectStacking: true,
    });
  }

  setView(v: View) { this.c.setViewportTransform([v.scale, 0, 0, v.scale, v.tx, v.ty]); }
  render() { this.c.renderAll(); }

  screenToImage(p: Pt): Pt {
    const q = util.transformPoint(new Point(p.x, p.y), util.invertTransform(this.c.viewportTransform));
    return { x: q.x, y: q.y };
  }
  imageToScreen(p: Pt): Pt {
    const q = util.transformPoint(new Point(p.x, p.y), this.c.viewportTransform);
    return { x: q.x, y: q.y };
  }

  addTile(key: string, bmp: ImageBitmap, x: number, y: number, w: number, h: number, z: 0 | 1) {
    // FabricImage's types want an HTMLImageElement/canvas; an ImageBitmap works for drawImage.
    const o = new FabricImage(bmp as unknown as HTMLCanvasElement, { ...STATIC, left: x, top: y, scaleX: w / bmp.width, scaleY: h / bmp.height });
    if (z === 0) { this.c.insertAt(0, o); this.nBase++; } else { this.c.insertAt(this.nBase + this.nTop, o); this.nTop++; }
    this.tiles.set(key, o);
  }
  removeTile(key: string) {
    const o = this.tiles.get(key);
    if (!o) return;
    const idx = this.c.getObjects().indexOf(o);
    this.c.remove(o); this.tiles.delete(key);
    if (idx < this.nBase) this.nBase--; else this.nTop--;
  }

  addMask(surfaces: Surface[], opacity: number) {
    for (const s of surfaces) {
      const o = new FabricImage(s.canvas, { ...STATIC, left: s.x, top: s.y, opacity });
      this.c.add(o); this.masks.set(s, o);
    }
  }
  updateMask(_dirty: Surface[]) { /* objectCaching off: the live canvas is drawn each render */ }

  addPatch(img: CanvasImageSource & { width: number; height: number }, p: PatchProps) {
    this.patch = new FabricImage(img as HTMLCanvasElement, { originX: 'center', originY: 'center', objectCaching: false });
    this.c.add(this.patch);
    this.setPatch(p);
  }
  setPatch(p: Partial<PatchProps>) {
    const o = this.patch!;
    if (p.cx !== undefined) o.set('left', p.cx);
    if (p.cy !== undefined) o.set('top', p.cy);
    if (p.scale !== undefined) o.set({ scaleX: p.scale, scaleY: p.scale });
    if (p.rotation !== undefined) o.set('angle', p.rotation);
    if (p.opacity !== undefined) o.set('opacity', p.opacity);
    if (p.visible !== undefined) o.set('visible', p.visible);
    o.setCoords();
  }
  patchCorners(): Pt[] {
    // Object space for a centre-origin FabricImage is centred on the object: corners at ±w/2, ±h/2.
    const o = this.patch!, m = o.calcTransformMatrix(), w = o.width / 2, h = o.height / 2;
    return [[-w, -h], [w, -h], [w, h], [-w, h]].map(([x, y]) => { const q = util.transformPoint(new Point(x, y), m); return { x: q.x, y: q.y }; });
  }
  enableHandles(on: boolean) {
    const o = this.patch!;
    o.set({ selectable: on, evented: on, hasControls: on });
    if (on) this.c.setActiveObject(o); else this.c.discardActiveObject();
    return { move: true, scale: true, rotate: true, builtIn: true };
  }
  setInteractive(on: boolean) { this.c.skipTargetFind = !on; }
  info() { return { version, objects: this.c.getObjects().length, canvases: 2 }; }
  destroy() { void this.c.dispose(); }
}
