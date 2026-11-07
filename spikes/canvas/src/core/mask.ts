// Mask storage at full image resolution over a region. Alpha channel = mask value (0..255).
// Display colour is red; the adapter shows the surfaces at reduced opacity.
// Two storage modes: one region-sized canvas, or 512 px tiles (only dirty tiles re-uploaded).
import { Pt, Rect } from './view';

export interface Surface { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D; x: number; y: number; w: number; h: number; dirty: boolean }
export type MaskStorage = 'region' | 'tiles';

export class MaskStore {
  readonly surfaces: Surface[] = [];
  private stamps = new Map<string, HTMLCanvasElement>();

  constructor(readonly region: Rect, readonly storage: MaskStorage, tile = 512) {
    const mk = (x: number, y: number, w: number, h: number) => {
      const canvas = document.createElement('canvas');
      canvas.width = w; canvas.height = h;
      const ctx = canvas.getContext('2d')!;
      this.surfaces.push({ canvas, ctx, x, y, w, h, dirty: false });
    };
    if (storage === 'region') mk(region.x, region.y, region.w, region.h);
    else for (let y = 0; y < region.h; y += tile) for (let x = 0; x < region.w; x += tile)
      mk(region.x + x, region.y + y, Math.min(tile, region.w - x), Math.min(tile, region.h - y));
  }

  /** Soft round stamp: alpha 1 inside hardness·r, linear falloff to 0 at r (the feather). */
  private stamp(r: number, hardness: number): HTMLCanvasElement {
    const key = `${r}:${hardness}`;
    let s = this.stamps.get(key);
    if (!s) {
      s = document.createElement('canvas');
      s.width = s.height = Math.ceil(2 * r);
      const c = s.getContext('2d')!;
      const g = c.createRadialGradient(r, r, r * hardness, r, r, r);
      g.addColorStop(0, 'rgba(255,0,0,1)'); g.addColorStop(1, 'rgba(255,0,0,0)');
      c.fillStyle = g; c.fillRect(0, 0, s.width, s.height);
      this.stamps.set(key, s);
    }
    return s;
  }

  /** Stamp at image point p. */
  dab(p: Pt, r: number, hardness: number, erase: boolean) {
    const st = this.stamp(r, hardness);
    for (const s of this.surfaces) {
      if (p.x + r < s.x || p.x - r > s.x + s.w || p.y + r < s.y || p.y - r > s.y + s.h) continue;
      s.ctx.globalCompositeOperation = erase ? 'destination-out' : 'source-over';
      s.ctx.drawImage(st, p.x - r - s.x, p.y - r - s.y);
      s.dirty = true;
    }
  }

  /** Stamps from a to b at spacing 0.25·r (a excluded). */
  segment(a: Pt, b: Pt, r: number, hardness: number, erase: boolean) {
    const d = Math.hypot(b.x - a.x, b.y - a.y), step = Math.max(1, r * 0.25), n = Math.max(1, Math.ceil(d / step));
    for (let i = 1; i <= n; i++) this.dab({ x: a.x + ((b.x - a.x) * i) / n, y: a.y + ((b.y - a.y) * i) / n }, r, hardness, erase);
  }

  takeDirty(): Surface[] {
    const d = this.surfaces.filter(s => s.dirty);
    d.forEach(s => (s.dirty = false));
    return d;
  }
}
