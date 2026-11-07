// In-page benchmark harness, exposed as window.bench and driven by Playwright (tests/) or ?autorun.
import { KonvaAdapter } from './adapters/konva';
import { FabricAdapter } from './adapters/fabric';
import { PixiAdapter } from './adapters/pixi';
import type { Adapter, LibName, PatchProps } from './adapters/types';
import { centerOn, fit, imageToScreen, screenToImage, zoomAbout, type Pt, type Rect, type View } from './core/view';
import { httpFetcher, TileManager, type Manifest, type TileFetcher } from './core/tiles';
import { MaskStore, type MaskStorage } from './core/mask';

export const W = 1600, H = 900;
const raf = () => new Promise<number>(r => requestAnimationFrame(r));
const pct = (a: number[], p: number) => { if (!a.length) return NaN; const s = [...a].sort((x, y) => x - y); return +s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))].toFixed(2); };
const stats = (iv: number[], cpu: number[]) => ({
  frames: iv.length, fps: +(1000 / (iv.reduce((a, b) => a + b, 0) / iv.length)).toFixed(1),
  frameP50: pct(iv, 50), frameP95: pct(iv, 95), frameP99: pct(iv, 99), frameMax: pct(iv, 100),
  over33ms: iv.filter(x => x > 33.4).length, cpuP50: pct(cpu, 50), cpuP95: pct(cpu, 95), cpuMax: pct(cpu, 100),
});

export const isTauri = () => '__TAURI_INTERNALS__' in window;
async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}
const schemeBase = (size: number) => (navigator.userAgent.includes('Windows') ? `http://tiles.localhost/${size}` : `tiles://localhost/${size}`);
const b64ToBytes = (s: string) => Uint8Array.from(atob(s), c => c.charCodeAt(0));

export type TileSrc = 'http' | 'scheme' | 'ipc-b64' | 'ipc-raw';
function fetcherFor(src: TileSrc, size: number, ext: string): TileFetcher {
  if (src === 'http') return httpFetcher(`/fixtures/tiles-${size}`, ext);
  if (src === 'scheme') return httpFetcher(schemeBase(size), ext);
  if (src === 'ipc-b64') return async (l, c, r) => createImageBitmap(new Blob([b64ToBytes(await tauriInvoke<string>('tile_b64', { size, level: l, col: c, row: r }))]));
  return async (l, c, r) => createImageBitmap(new Blob([await tauriInvoke<ArrayBuffer>('tile_raw', { size, level: l, col: c, row: r })]));
}

class Bench {
  a?: Adapter;
  tm?: TileManager;
  view: View = { scale: 1, tx: 0, ty: 0 };
  size = 0;
  mask?: MaskStore;
  busy = false;
  log: string[] = [];

  async setup(lib: LibName) {
    this.teardown();
    const el = document.getElementById('stage')!;
    el.querySelectorAll(':scope > :not(#overlay)').forEach(n => n.remove());
    const host = document.createElement('div');
    host.style.cssText = 'position:absolute;inset:0';
    el.prepend(host);
    this.a = lib === 'konva' ? new KonvaAdapter() : lib === 'fabric' ? new FabricAdapter() : new PixiAdapter();
    await this.a.init(host, W, H);
    this.a.setInteractive(false);
    return this.a.info();
  }

  teardown() { this.tm?.destroy(); this.tm = undefined; this.mask = undefined; this.a?.destroy(); this.a = undefined; }

  apply(v: View) { this.view = v; this.tm?.update(v, W, H); this.a!.setView(v); }

  /** Canvas/WebGL limits of this engine. A 2D canvas "works" if a pixel drawn at its far corner reads back. */
  limits() {
    const probe2d = (w: number, h: number) => {
      try {
        const c = document.createElement('canvas'); c.width = w; c.height = h;
        const x = c.getContext('2d', { willReadFrequently: true });
        if (!x) return false;
        x.fillStyle = '#f00'; x.fillRect(w - 1, h - 1, 1, 1);
        const ok = x.getImageData(w - 1, h - 1, 1, 1).data[0] === 255;
        c.width = c.height = 0;
        return ok;
      } catch { return false; }
    };
    const dims: [number, number][] = [[16384, 16384], [16385, 16384], [32767, 8192], [32768, 8192], [32767, 1], [32768, 1], [65535, 1], [65536, 1], [65535, 4096], [23170, 11585], [20000, 13421], [20000, 13422]];
    const canvas2d = Object.fromEntries(dims.map(([w, h]) => [`${w}x${h}`, probe2d(w, h)]));
    const gl = document.createElement('canvas').getContext('webgl2')!;
    const dbg = gl.getExtension('WEBGL_debug_renderer_info');
    return {
      userAgent: navigator.userAgent,
      canvas2d,
      webgl2: {
        maxTextureSize: gl.getParameter(gl.MAX_TEXTURE_SIZE), maxRenderbufferSize: gl.getParameter(gl.MAX_RENDERBUFFER_SIZE),
        maxViewportDims: Array.from(gl.getParameter(gl.MAX_VIEWPORT_DIMS) as Int32Array),
        renderer: dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
      },
      webgpu: 'gpu' in navigator,
      deviceMemoryGB: (navigator as unknown as { deviceMemory?: number }).deviceMemory,
    };
  }

  async webgpuLimits() {
    const gpu = (navigator as unknown as { gpu?: { requestAdapter(): Promise<{ limits: Record<string, number> } | null> } }).gpu;
    const ad = await gpu?.requestAdapter();
    return ad ? { maxTextureDimension2D: ad.limits.maxTextureDimension2D, maxBufferSize: ad.limits.maxBufferSize } : null;
  }

  /** Single image: fetch + decode the whole JPEG into one ImageBitmap, show it fitted. */
  async loadSingle(size: number) {
    this.size = size;
    const t0 = performance.now();
    const blob = await (await fetch(`/fixtures/map-${size}.jpg`)).blob();
    const t1 = performance.now();
    let bmp: ImageBitmap;
    try { bmp = await createImageBitmap(blob); } catch (e) { return { ok: false, stage: 'decode', error: String(e) }; }
    const t2 = performance.now();
    try {
      this.a!.addTile('single', bmp, 0, 0, size, size, 0);
      this.apply(fit({ x: 0, y: 0, w: size, h: size }, W, H));
      this.a!.render();
    } catch (e) { return { ok: false, stage: 'add', error: String(e) }; }
    const t3 = performance.now();
    await raf(); await raf();
    const t4 = performance.now();
    return { ok: true, fetchMs: +(t1 - t0).toFixed(1), decodeMs: +(t2 - t1).toFixed(1), firstRenderCpuMs: +(t3 - t2).toFixed(1), ttfvMs: +(t4 - t0).toFixed(1), firstFrameMs: +(t4 - t2).toFixed(1) };
  }

  async loadTiles(size: number, src: TileSrc = 'http') {
    this.size = size;
    const t0 = performance.now();
    const m: Manifest = src === 'http' ? await (await fetch(`/fixtures/tiles-${size}/manifest.json`)).json()
      : src === 'scheme' ? await (await fetch(`${schemeBase(size)}/manifest.json`)).json()
      : JSON.parse(await tauriInvoke<string>('manifest', { size }));
    this.tm = new TileManager(m, fetcherFor(src, size, m.ext), this.a!);
    const v = fit({ x: 0, y: 0, w: size, h: size }, W, H);
    let frames = 0;
    do { this.apply(v); this.a!.render(); await raf(); frames++; } while (!this.tm.settled && frames < 2000);
    await raf();
    return { ok: this.tm.settled, ttfvMs: +(performance.now() - t0).toFixed(1), tilesShown: this.tm.shownCount, loads: this.tm.loads, errors: this.tm.errors };
  }

  /** Time-based scripted pan/zoom (identical path for every library). 9 s. */
  async panZoom(durationScale = 1) {
    const N = this.size, cx = N / 2, cy = N / 2, f = fit({ x: 0, y: 0, w: N, h: N }, W, H).scale;
    const T = [3000, 6000, 7500, 9000].map(t => t * durationScale);
    const clamp = (v: number) => Math.min(N, Math.max(0, v));
    const at = (t: number): View => {
      if (t < T[0]) { const u = t / T[0]; return centerOn(f * (1 / f) ** u, cx, cy, W / 2, H / 2); }
      if (t < T[1]) { const u = (t - T[0]) / (T[1] - T[0]); return centerOn(1, clamp(cx + 1500 * u), clamp(cy + 600 * u), W / 2, H / 2); }
      const px = clamp(cx + 1500), py = clamp(cy + 600);
      if (t < T[2]) { const u = (t - T[1]) / (T[2] - T[1]); return centerOn(0.125 ** u, px, py, W / 2, H / 2); }
      const u = Math.min(1, (t - T[2]) / (T[3] - T[2]));
      return centerOn(0.125, clamp(px - 8000 * u), py, W / 2, H / 2);
    };
    const iv: number[] = [], cpu: number[] = [], long: { t: number; dt: number; prevCpu: number; prevApply: number; shown: number }[] = [];
    let prevApply = 0;
    const loads0 = this.tm?.loads ?? 0;
    this.busy = true;
    let start = 0, last = 0, lastApply = 0;
    await new Promise<void>(done => {
      const tick = (t: number) => {
        if (!start) start = t; else iv.push(t - last);
        last = t;
        const c0 = performance.now();
        this.apply(at(t - start)); const ca = performance.now(); this.a!.render(); lastApply = ca - c0;
        cpu.push(performance.now() - c0);
        if (iv.length && iv[iv.length - 1] > 50) long.push({ t: Math.round(t - start), dt: Math.round(iv[iv.length - 1]), prevCpu: +(cpu[cpu.length - 2] ?? 0).toFixed(1), prevApply: +prevApply.toFixed(1), shown: this.tm?.shownCount ?? 1 });
        prevApply = lastApply;
        if (t - start < T[3]) requestAnimationFrame(tick); else done();
      };
      requestAnimationFrame(tick);
    });
    this.busy = false;
    return { ...stats(iv, cpu), long, tileLoads: (this.tm?.loads ?? 0) - loads0, tileErrors: this.tm?.errors ?? 0, info: this.a!.info() };
  }

  /** Mask painting over a region at full resolution: scripted zig-zag strokes, one segment per frame. */
  async maskPaint(o: { region: Rect; storage: MaskStorage; radius: number; hardness: number; frames: number }) {
    this.mask = new MaskStore(o.region, o.storage);
    this.a!.addMask(this.mask.surfaces, 0.5);
    this.apply(fit(o.region, W, H, 20));
    for (let i = 0; i < 30 && this.tm && !this.tm.settled; i++) { this.a!.render(); await raf(); this.apply(this.view); }
    const r = o.region, step = o.radius * 0.6;
    let p: Pt = { x: r.x + o.radius, y: r.y + o.radius }, dir = 1, row = 0;
    const iv: number[] = [], cpu: number[] = [], dabCpu: number[] = [];
    let last = 0, n = 0;
    this.busy = true;
    await new Promise<void>(done => {
      const tick = (t: number) => {
        if (last) iv.push(t - last);
        last = t;
        const c0 = performance.now();
        let q = { x: p.x + dir * step, y: p.y };
        if (q.x > r.x + r.w - o.radius || q.x < r.x + o.radius) { dir = -dir; row++; q = { x: p.x, y: r.y + o.radius + ((row * o.radius * 1.5) % (r.h - 2 * o.radius)) }; }
        const erase = Math.floor(n / 40) % 3 === 2; // every third 40-frame stroke erases
        this.mask!.segment(p, q, o.radius, o.hardness, erase);
        p = q;
        const c1 = performance.now();
        this.a!.updateMask(this.mask!.takeDirty());
        this.a!.render();
        const c2 = performance.now();
        dabCpu.push(c1 - c0); cpu.push(c2 - c0);
        if (++n < o.frames) requestAnimationFrame(tick); else done();
      };
      requestAnimationFrame(tick);
    });
    this.busy = false;
    // Export: region-sized 8-bit mask (alpha) as the provider would need it.
    const e0 = performance.now();
    const out = document.createElement('canvas'); out.width = r.w; out.height = r.h;
    const ox = out.getContext('2d', { willReadFrequently: true })!;
    for (const s of this.mask.surfaces) ox.drawImage(s.canvas, s.x - r.x, s.y - r.y);
    const px = ox.getImageData(0, 0, r.w, r.h).data;
    let painted = 0;
    for (let i = 3; i < px.length; i += 4) if (px[i] > 0) painted++;
    const exportMs = performance.now() - e0;
    return { ...stats(iv, cpu), dabP95: pct(dabCpu, 95), exportMs: +exportMs.toFixed(1), paintedFraction: +(painted / (r.w * r.h)).toFixed(3), surfaces: this.mask.surfaces.length, info: this.a!.info() };
  }

  /** Screen ↔ image round trip via the library's own transform, at several zooms. */
  coords() {
    let seed = 7;
    const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    const zooms = [1 / 64, 1 / 16, 1 / 4, 0.3719, 1, 2.5, 8, 32];
    const rows = zooms.map(z => {
      const v = centerOn(z, 8191.3, 4096.7, 800.5, 450.25);
      this.a!.setView(v);
      let roundTrip = 0, vsCore = 0, pixelMismatch = 0;
      for (let i = 0; i < 500; i++) {
        const p = { x: rnd() * 16384, y: rnd() * 16384 };
        const s = this.a!.imageToScreen(p), q = this.a!.screenToImage(s), c = imageToScreen(v, p);
        roundTrip = Math.max(roundTrip, Math.hypot(q.x - p.x, q.y - p.y));
        vsCore = Math.max(vsCore, Math.hypot(s.x - c.x, s.y - c.y));
        // pixel identity: screen pixel centre → image pixel → its centre back to screen → same screen pixel (zoom ≥ 1)
        const sp = { x: Math.floor(rnd() * W) + 0.5, y: Math.floor(rnd() * H) + 0.5 };
        const ip = this.a!.screenToImage(sp), pix = { x: Math.floor(ip.x), y: Math.floor(ip.y) };
        const back = this.a!.imageToScreen({ x: pix.x + 0.5, y: pix.y + 0.5 });
        if (z >= 1 && (Math.abs(back.x - sp.x) > z / 2 + 1e-9 || Math.abs(back.y - sp.y) > z / 2 + 1e-9)) pixelMismatch++;
      }
      return { zoom: +z.toFixed(4), roundTripMaxPx: roundTrip, vsCoreMaxScreenPx: vsCore, pixelMismatch };
    });
    // Anchored zoom drift: 2000 wheel steps at random cursors; the image point under the cursor must not move.
    let v: View = fit({ x: 0, y: 0, w: 16384, h: 16384 }, W, H), anchorDrift = 0;
    for (let i = 0; i < 2000; i++) {
      const sx = rnd() * W, sy = rnd() * H;
      this.a!.setView(v);
      const before = this.a!.screenToImage({ x: sx, y: sy });
      v = zoomAbout(v, rnd() < 0.5 ? 1.1 : 1 / 1.1, sx, sy);
      this.a!.setView(v);
      const after = this.a!.screenToImage({ x: sx, y: sy });
      if (v.scale > 1 / 128 && v.scale < 64) anchorDrift = Math.max(anchorDrift, Math.hypot(after.x - before.x, after.y - before.y));
    }
    // Return trip: 500 zoom-ins then 500 zoom-outs about one point → view must come back.
    const v0 = centerOn(0.5, 5000.25, 7000.75, 800, 450);
    let w = v0;
    for (let i = 0; i < 500; i++) w = zoomAbout(w, 1.01, 333.3, 222.2, 1e-9, 1e9);
    for (let i = 0; i < 500; i++) w = zoomAbout(w, 1 / 1.01, 333.3, 222.2, 1e-9, 1e9);
    const returnErr = { scale: Math.abs(w.scale - v0.scale) / v0.scale, tx: Math.abs(w.tx - v0.tx), ty: Math.abs(w.ty - v0.ty) };
    this.a!.setView(this.view);
    return { rows, anchorDriftMaxImagePx: anchorDrift, returnTrip: returnErr };
  }

  /** Rendered alignment: a 64² probe with a unique colour per pixel is placed far from the origin;
   *  returns where each probe pixel centre should land on screen so the test can sample a screenshot. */
  async probe(zoom: number, at = 16000) {
    const c = document.createElement('canvas'); c.width = c.height = 64;
    const x = c.getContext('2d')!, id = x.createImageData(64, 64);
    for (let j = 0; j < 64; j++) for (let i = 0; i < 64; i++) { const o = (j * 64 + i) * 4; id.data.set([i * 4, j * 4, 128 + ((i + j) & 1) * 100, 255], o); }
    x.putImageData(id, 0, 0);
    const bmp = await createImageBitmap(c);
    this.a!.removeTile('probe');
    this.a!.addTile('probe', bmp, at, at, 64, 64, 1);
    const v = centerOn(zoom, at + 32.3, at + 32.7, W / 2 + 0.37, H / 2 + 0.61);
    this.view = v; this.a!.setView(v); this.a!.render(); await raf(); await raf();
    const samples: { sx: number; sy: number; r: number; g: number; b: number }[] = [];
    for (let j = 0; j < 64; j++) for (let i = 0; i < 64; i++) {
      const s = this.a!.imageToScreen({ x: at + i + 0.5, y: at + j + 0.5 });
      if (s.x < 0 || s.y < 0 || s.x >= W || s.y >= H) continue;
      samples.push({ sx: Math.floor(s.x), sy: Math.floor(s.y), r: i * 4, g: j * 4, b: 128 + ((i + j) & 1) * 100 });
    }
    return samples;
  }

  /** Patch layer: set a transform, read the corners back from the library, compare with the math. */
  async layers() {
    const bmp = await createImageBitmap(await (await fetch('/fixtures/patch.png')).blob());
    const p: PatchProps = { cx: 3000.5, cy: 2000.25, scale: 1.25, rotation: 30, opacity: 0.6, visible: true };
    this.a!.addPatch(bmp, p);
    const got = this.a!.patchCorners();
    const th = (p.rotation * Math.PI) / 180, hw = (bmp.width * p.scale) / 2, hh = (bmp.height * p.scale) / 2;
    const exp = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(([x, y]) => ({ x: p.cx + x * Math.cos(th) - y * Math.sin(th), y: p.cy + x * Math.sin(th) + y * Math.cos(th) }));
    const cornerErr = Math.max(...got.map((g, i) => Math.hypot(g.x - exp[i].x, g.y - exp[i].y)));
    this.a!.setPatch({ visible: false }); this.a!.render();
    this.a!.setPatch({ visible: true, opacity: 0.3 }); this.a!.render();
    const handles = this.a!.enableHandles(true);
    this.apply(fit({ x: 2000, y: 1000, w: 2000, h: 2000 }, W, H));
    this.a!.setInteractive(true);
    this.a!.render();
    return { cornerErrPx: cornerErr, handles };
  }

  mem() {
    const m = (performance as unknown as { memory?: { usedJSHeapSize: number; totalJSHeapSize: number } }).memory;
    return m ? { jsHeapUsedMB: +(m.usedJSHeapSize / 2 ** 20).toFixed(1), jsHeapTotalMB: +(m.totalJSHeapSize / 2 ** 20).toFixed(1) } : null;
  }

  /** Tauri only: tile transport cost (custom scheme vs IPC base64 vs IPC raw bytes). */
  async transfer(size: number, n = 200) {
    const m: Manifest = JSON.parse(await tauriInvoke<string>('manifest', { size }));
    const lv = m.levels[m.maxLevel], tiles: [number, number][] = [];
    for (let i = 0; i < n; i++) tiles.push([i % lv.cols, Math.floor(i / lv.cols) % lv.rows]);
    const get: Record<string, (c: number, r: number) => Promise<Uint8Array<ArrayBuffer>>> = {
      scheme: async (c, r) => new Uint8Array(await (await fetch(`${schemeBase(size)}/${m.maxLevel}/${c}_${r}.${m.ext}`)).arrayBuffer()),
      'ipc-b64': async (c, r) => b64ToBytes(await tauriInvoke<string>('tile_b64', { size, level: m.maxLevel, col: c, row: r })),
      'ipc-raw': async (c, r) => new Uint8Array(await tauriInvoke<ArrayBuffer>('tile_raw', { size, level: m.maxLevel, col: c, row: r })),
    };
    const out: Record<string, unknown> = {};
    for (const [k, fn] of Object.entries(get)) {
      await fn(0, 0); // warm
      let bytes = 0;
      const t0 = performance.now();
      for (const [c, r] of tiles) bytes += (await fn(c, r)).length;
      const seqMs = performance.now() - t0;
      const t1 = performance.now();
      for (let i = 0; i < tiles.length; i += 8) await Promise.all(tiles.slice(i, i + 8).map(([c, r]) => fn(c, r)));
      const parMs = performance.now() - t1;
      const t2 = performance.now();
      for (let i = 0; i < tiles.length; i += 8) await Promise.all(tiles.slice(i, i + 8).map(async ([c, r]) => (await createImageBitmap(new Blob([await fn(c, r)]))).close()));
      const decMs = performance.now() - t2;
      out[k] = { tiles: n, avgKB: +(bytes / n / 1024).toFixed(1), seqMsPerTile: +(seqMs / n).toFixed(2), par8MsPerTile: +(parMs / n).toFixed(2), par8WithDecodeMsPerTile: +(decMs / n).toFixed(2), seqMBps: +(bytes / 2 ** 20 / (seqMs / 1000)).toFixed(1) };
    }
    return out;
  }

  async report(json: unknown) { if (isTauri()) await tauriInvoke('report', { json: JSON.stringify(json) }); }

  /** Diagnostic: cost of showing n new 512² tiles in one render, by ImageBitmap options / source type. */
  async uploadProbe(n: number, kind: 'default' | 'premul' | 'none' | 'nocs' | 'img', offset = 0) {
    const blobs = await Promise.all(Array.from({ length: n }, (_, i) => fetch(`/fixtures/tiles-16384/14/${(i + offset) % 32}_${Math.floor((i + offset) / 32)}.jpeg`).then(r => r.blob())));
    const srcs = await Promise.all(blobs.map(async b => {
      if (kind === 'img') { const im = new Image(); im.src = URL.createObjectURL(b); await im.decode(); return im as unknown as ImageBitmap; }
      const o: ImageBitmapOptions = kind === 'premul' ? { premultiplyAlpha: 'premultiply' } : kind === 'none' ? { premultiplyAlpha: 'none' } : kind === 'nocs' ? { colorSpaceConversion: 'none', premultiplyAlpha: 'none' } : {};
      return createImageBitmap(b, o);
    }));
    this.apply({ scale: 0.05, tx: 0, ty: 0 });
    srcs.forEach((s, i) => this.a!.addTile(`u${i}`, s, (i % 8) * 10, Math.floor(i / 8) * 10, 10, 10, 1));
    const t0 = performance.now(); this.a!.render(); const cpu = performance.now() - t0;
    await raf(); const frame = performance.now() - t0;
    srcs.forEach((_, i) => this.a!.removeTile(`u${i}`));
    return { kind, n, renderCpuMs: +cpu.toFixed(1), toNextFrameMs: +frame.toFixed(1) };
  }

  screenToImage(p: Pt) { return screenToImage(this.view, p); }
}

export const bench = new Bench();
(window as unknown as { bench: Bench }).bench = bench;
