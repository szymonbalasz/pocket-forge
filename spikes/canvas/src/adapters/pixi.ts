import { Application, CanvasSource, Container, ImageSource, Sprite, Texture, VERSION, type FederatedPointerEvent } from 'pixi.js';
import type { Adapter, PatchProps } from './types';
import type { Pt, View } from '../core/view';
import type { Surface } from '../core/mask';

export class PixiAdapter implements Adapter {
  readonly name = 'pixi' as const;
  private app!: Application;
  private world = new Container();
  private base = new Container();
  private top = new Container();
  private overlay = new Container();
  private tiles = new Map<string, Sprite>();
  private masks = new Map<Surface, Sprite>();
  private patch?: Sprite;
  private drag?: { dx: number; dy: number };
  maxTextureSize = 0;
  /** Diagnostic toggle (window.__pixiKeep): keep GL textures for hidden tiles and reuse them, instead of deleting. */
  private keep = !!(window as unknown as { __pixiKeep?: boolean }).__pixiKeep;
  private texCache = new Map<ImageBitmap, Texture>();
  uploads = 0;

  async init(el: HTMLElement, w: number, h: number) {
    this.app = new Application();
    await this.app.init({ width: w, height: h, preference: 'webgl', autoStart: false, antialias: false, background: '#202020', resolution: 1 });
    el.append(this.app.canvas);
    this.world.addChild(this.base, this.top, this.overlay);
    this.app.stage.addChild(this.world);
    const gl = (this.app.renderer as unknown as { gl: WebGL2RenderingContext }).gl;
    this.maxTextureSize = gl.getParameter(gl.MAX_TEXTURE_SIZE);
    this.app.ticker.stop();
  }

  setView(v: View) { this.world.scale.set(v.scale); this.world.position.set(v.tx, v.ty); }
  render() { this.app.renderer.render(this.app.stage); }

  screenToImage(p: Pt): Pt { const q = this.world.toLocal(p); return { x: q.x, y: q.y }; }
  imageToScreen(p: Pt): Pt { const q = this.world.toGlobal(p); return { x: q.x, y: q.y }; }

  addTile(key: string, bmp: ImageBitmap, x: number, y: number, w: number, h: number, z: 0 | 1) {
    if (Math.max(bmp.width, bmp.height) > this.maxTextureSize) throw new Error(`texture ${bmp.width}×${bmp.height} > MAX_TEXTURE_SIZE ${this.maxTextureSize}`);
    // A fresh source per tile (Texture.from would cache by resource globally).
    let tex = this.keep ? this.texCache.get(bmp) : undefined;
    if (!tex) { tex = new Texture({ source: new ImageSource({ resource: bmp }) }); if (this.keep) this.texCache.set(bmp, tex); }
    const s = new Sprite(tex);
    s.position.set(x, y); s.width = w; s.height = h;
    (z ? this.top : this.base).addChild(s);
    this.tiles.set(key, s);
  }
  removeTile(key: string) {
    const s = this.tiles.get(key);
    if (!s) return;
    if (this.keep) { s.destroy(); this.tiles.delete(key); return; }
    // Free the GPU texture now; the ImageBitmap stays in the tile cache.
    s.texture.source.unload();
    s.destroy({ texture: true });
    this.tiles.delete(key);
  }

  addMask(surfaces: Surface[], opacity: number) {
    for (const s of surfaces) {
      const sp = new Sprite(new Texture({ source: new CanvasSource({ resource: s.canvas }) }));
      sp.position.set(s.x, s.y); sp.alpha = opacity;
      this.overlay.addChild(sp); this.masks.set(s, sp);
    }
  }
  /** WebGL can't sample a 2D canvas directly: each dirty surface is re-uploaded whole (texImage2D). */
  updateMask(dirty: Surface[]) {
    for (const s of dirty) { this.masks.get(s)!.texture.source.update(); this.uploads++; }
  }

  addPatch(img: CanvasImageSource & { width: number; height: number }, p: PatchProps) {
    const src = img instanceof HTMLCanvasElement ? new CanvasSource({ resource: img }) : new ImageSource({ resource: img as ImageBitmap });
    this.patch = new Sprite(new Texture({ source: src }));
    this.patch.anchor.set(0.5);
    this.overlay.addChild(this.patch);
    this.setPatch(p);
  }
  setPatch(p: Partial<PatchProps>) {
    const s = this.patch!;
    if (p.cx !== undefined) s.x = p.cx;
    if (p.cy !== undefined) s.y = p.cy;
    if (p.scale !== undefined) s.scale.set(p.scale);
    if (p.rotation !== undefined) s.angle = p.rotation;
    if (p.opacity !== undefined) s.alpha = p.opacity;
    if (p.visible !== undefined) s.visible = p.visible;
  }
  patchCorners(): Pt[] {
    const s = this.patch!, w = s.texture.width, h = s.texture.height;
    s.updateLocalTransform();
    return [[-w / 2, -h / 2], [w / 2, -h / 2], [w / 2, h / 2], [-w / 2, h / 2]].map(([x, y]) => { const q = s.localTransform.apply({ x, y }); return { x: q.x, y: q.y }; });
  }
  /** No transformer in Pixi: only drag-to-move is wired here (handles would be custom work). */
  enableHandles(on: boolean) {
    const s = this.patch!;
    s.eventMode = on ? 'static' : 'none';
    s.cursor = 'move';
    if (on) {
      s.on('pointerdown', (e: FederatedPointerEvent) => { const q = this.world.toLocal(e.global); this.drag = { dx: s.x - q.x, dy: s.y - q.y }; });
      this.app.stage.eventMode = 'static';
      this.app.stage.hitArea = this.app.screen;
      this.app.stage.on('globalpointermove', (e: FederatedPointerEvent) => { if (!this.drag) return; const q = this.world.toLocal(e.global); s.position.set(q.x + this.drag.dx, q.y + this.drag.dy); });
      this.app.stage.on('pointerup', () => (this.drag = undefined));
    } else s.removeAllListeners();
    return { move: true, scale: false, rotate: false, builtIn: false };
  }
  setInteractive(on: boolean) { this.app.stage.eventMode = on ? 'static' : 'none'; }
  info() { return { version: VERSION, sprites: this.tiles.size, maxTextureSize: this.maxTextureSize, maskUploads: this.uploads, keepTextures: this.keep }; }
  destroy() { this.app.destroy(true, { children: true, texture: true }); }
}
