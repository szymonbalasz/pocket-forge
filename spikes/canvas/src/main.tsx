import { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { bench, H, isTauri, W, type TileSrc } from './bench';
import { panBy, zoomAbout, type Pt } from './core/view';
import { MaskStore } from './core/mask';
import type { LibName } from './adapters/types';

const q = new URLSearchParams(location.search);
// In Tauri the window URL is fixed; autorun params come from the Rust side (SPIKE_QUERY).
if (isTauri()) {
  const { invoke } = await import('@tauri-apps/api/core');
  new URLSearchParams(await invoke<string>('autorun')).forEach((v, k) => q.set(k, v));
}
type Tool = 'pan' | 'mask' | 'layer';

function App() {
  const [lib, setLib] = useState<LibName>((q.get('lib') as LibName) ?? 'konva');
  const [mode, setMode] = useState(q.get('mode') ?? 'tiles');
  const [size, setSize] = useState(Number(q.get('size') ?? 8192));
  const [tool, setTool] = useState<Tool>('pan');
  const [status, setStatus] = useState('');
  const overlay = useRef<HTMLDivElement>(null);
  const manual = !q.has('bench') && !q.has('autorun');

  useEffect(() => {
    if (!manual) return;
    let stop = false;
    (async () => {
      setStatus('loading…');
      await bench.setup(lib);
      const src: TileSrc = isTauri() ? 'scheme' : 'http';
      const r = mode === 'single' ? await bench.loadSingle(size) : await bench.loadTiles(size, src);
      setStatus(JSON.stringify(r));
      const loop = () => { if (stop) return; if (!bench.busy && bench.a) { bench.apply(bench.view); bench.a.render(); } requestAnimationFrame(loop); };
      loop();
    })();
    return () => { stop = true; };
  }, [lib, mode, size, manual]);

  useEffect(() => { bench.a?.setInteractive(tool === 'layer'); }, [tool]);

  // Pan/zoom/paint input on a transparent overlay (removed in layer mode so the library gets events).
  useEffect(() => {
    const el = overlay.current!;
    let drag: Pt | null = null;
    const local = (e: PointerEvent | WheelEvent): Pt => { const r = el.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
    const wheel = (e: WheelEvent) => { e.preventDefault(); const p = local(e); bench.view = zoomAbout(bench.view, e.deltaY < 0 ? 1.15 : 1 / 1.15, p.x, p.y); };
    const down = (e: PointerEvent) => {
      el.setPointerCapture(e.pointerId); drag = local(e);
      if (tool === 'mask' && bench.a && !bench.mask) {
        const c = bench.screenToImage(drag);
        bench.mask = new MaskStore({ x: Math.max(0, Math.round(c.x - 1024)), y: Math.max(0, Math.round(c.y - 1024)), w: 2048, h: 2048 }, 'region');
        bench.a.addMask(bench.mask.surfaces, 0.5);
      }
    };
    const move = (e: PointerEvent) => {
      if (!drag) return;
      const p = local(e);
      if (tool === 'pan') bench.view = panBy(bench.view, p.x - drag.x, p.y - drag.y);
      else if (tool === 'mask' && bench.mask) {
        bench.mask.segment(bench.screenToImage(drag), bench.screenToImage(p), 60, 0.5, e.shiftKey);
        bench.a!.updateMask(bench.mask.takeDirty());
      }
      drag = p;
    };
    const up = () => { drag = null; };
    el.addEventListener('wheel', wheel, { passive: false });
    el.addEventListener('pointerdown', down); el.addEventListener('pointermove', move); el.addEventListener('pointerup', up);
    return () => { el.removeEventListener('wheel', wheel); el.removeEventListener('pointerdown', down); el.removeEventListener('pointermove', move); el.removeEventListener('pointerup', up); };
  }, [tool]);

  return (
    <>
      {manual && (
        <div id="bar">
          <select value={lib} onChange={e => setLib(e.target.value as LibName)}>{['konva', 'fabric', 'pixi'].map(l => <option key={l}>{l}</option>)}</select>
          <select value={mode} onChange={e => setMode(e.target.value)}>{['tiles', 'single'].map(l => <option key={l}>{l}</option>)}</select>
          <select value={size} onChange={e => setSize(+e.target.value)}>{[8192, 16384].map(l => <option key={l}>{l}</option>)}</select>
          {(['pan', 'mask', 'layer'] as Tool[]).map(t => <label key={t}><input type="radio" checked={tool === t} onChange={() => setTool(t)} />{t}</label>)}
          <button onClick={async () => setStatus(JSON.stringify(await bench.layers()))}>add patch</button>
          <span style={{ opacity: 0.7 }}>{status}</span>
        </div>
      )}
      <div id="stage" style={{ width: W, height: H }}>
        <div id="overlay" ref={overlay} style={{ pointerEvents: tool === 'layer' ? 'none' : 'auto' }} />
      </div>
    </>
  );
}

createRoot(document.getElementById('root')!).render(<App />);

// Tauri smoke: ?autorun=tauri&lib=konva — runs transfer + tiles + pan/zoom + mask, reports to the Rust side.
if (q.get('autorun') === 'tauri') {
  setTimeout(async () => {
    const lib = (q.get('lib') as LibName) ?? 'konva', out: Record<string, unknown> = { lib, limits: bench.limits() };
    try {
      out.transfer16k = await bench.transfer(16384, 200);
      for (const src of ['scheme', 'ipc-b64'] as TileSrc[]) {
        await bench.setup(lib);
        out[`tiles16k_${src}`] = await bench.loadTiles(16384, src);
        out[`panZoom16k_${src}`] = await bench.panZoom();
      }
      out.mask = await bench.maskPaint({ region: { x: 6000, y: 6000, w: 2048, h: 2048 }, storage: 'region', radius: 60, hardness: 0.5, frames: 300 });
      out.mem = bench.mem();
    } catch (e) { out.error = String(e); }
    await bench.report(out);
  }, 500);
}
