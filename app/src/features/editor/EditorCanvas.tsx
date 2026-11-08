// The editor canvas (canvas slot): a Konva stage whose transform is the app-owned view
// (FR-EDT-06), the tile layer drawn by TileManager, wheel zoom, pan, presets and HiDPI.
import Konva from 'konva';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Layer, Stage } from 'react-konva';

import {
  api,
  onEvent,
  type Layer as ManifestLayer,
  type ProjectSnapshot,
  type TilesDescriptor,
} from '../../api';
import { actions } from './actions';
import { useTestHook } from './hook';
import { dispatchShortcut, isTextTarget, type ShortcutMap } from './shortcuts';
import { editorStore, useEditor } from './store';
import { editorBridge } from './bridge';
import { CompareDivider } from './CompareDivider';
import { CompareLayer } from './CompareLayer';
import { PatchLayers } from './PatchLayers';
import { useMaskTools } from './useMaskTools';
import { konvaSink, TileManager, type TileStats } from './TileManager';
import { fit, panBy, scaleLimits, wheelFactor, zoomAbout } from './view';

function useElementSize(ref: React.RefObject<HTMLElement | null>) {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => editorStore.set({ viewport: { w: el.clientWidth, h: el.clientHeight } });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
}

/** FR-EDT-05: follow devicePixelRatio changes (monitor move, Windows scaling) without reload.
 *  Checked every animation frame (one number compare), so the frame after a change already has
 *  the new pixel ratio and tile level. */
function useDevicePixelRatio(stage: React.RefObject<Konva.Stage | null>) {
  useEffect(() => {
    let raf = 0;
    let applied = 0;
    const tick = () => {
      const next = window.devicePixelRatio || 1;
      if (next !== applied) {
        applied = next;
        Konva.pixelRatio = next;
        for (const layer of stage.current?.getLayers() ?? []) {
          layer.getCanvas().setPixelRatio(next);
          layer.batchDraw();
        }
        if (next !== editorStore.get().dpr) editorStore.set({ dpr: next });
      }
      raf = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(raf);
  }, [stage]);
}

type Drag = { kind: 'pan'; x: number; y: number; pointerId: number };

export function EditorCanvas({ project }: { project: ProjectSnapshot }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<Konva.Stage>(null);
  // The layers are state (callback refs), not refs: the Stage remounts when the viewport drops
  // to 0 (the store resets on open), and every controller must rebind to the new layer (D1).
  const [tilesLayer, setTilesLayer] = useState<Konva.Layer | null>(null);
  const tilesRef = useRef<TileManager | null>(null);
  const [patchesLayer, setPatchesLayer] = useState<Konva.Layer | null>(null);
  const patchesRef = useRef<PatchLayers | null>(null);
  const [maskLayer, setMaskLayer] = useState<Konva.Layer | null>(null);
  const [compareLayer, setCompareLayer] = useState<Konva.Layer | null>(null);
  const compareRef = useRef<CompareLayer | null>(null);
  const [compareError, setCompareError] = useState<string | null>(null);
  const [stack, setStack] = useState<ManifestLayer[]>(project.layers);
  const [layerError, setLayerError] = useState<string | null>(null);
  const [desc, setDesc] = useState<TilesDescriptor | null>(null);
  const [tileStats, setTileStats] = useState<TileStats | null>(null);
  const [buildError, setBuildError] = useState<string | null>(null);
  const [spaceHeld, setSpaceHeld] = useState(false);
  const drag = useRef<Drag | null>(null);
  const { view, viewport, dpr, tool, image, maskVisible } = useEditor((s) => s);
  const { compare, compareMode, split, flipBefore } = useEditor((s) => s);
  const tools = useMaskTools({ project, maskLayer, container: containerRef });

  useElementSize(containerRef);
  useDevicePixelRatio(stageRef);

  const originalId = project.layers[0]?.id;

  // Open the pyramid (FR-EDT-07): the descriptor comes back at once; levels arrive by event.
  const openTiles = useCallback(
    async (rebuild: boolean) => {
      if (!originalId) return;
      setBuildError(null);
      try {
        const d = await api.editor.tilesOpen(project.project_id, originalId, rebuild);
        setDesc(d);
        if (rebuild) tilesRef.current?.reset(d);
      } catch (e) {
        setBuildError((e as Error).message);
      }
    },
    [project.project_id, originalId],
  );

  useEffect(() => {
    editorStore.reset();
    editorStore.set({ image: { w: project.width, h: project.height } });
    void openTiles(false);
    return () => editorStore.set({ image: null });
  }, [project.project_id, project.width, project.height, openTiles]);

  // Tile events for this pyramid.
  useEffect(() => {
    if (!desc) return;
    const subs = [
      onEvent('edt:tiles-progress', (p) => {
        if (p.key === tilesRef.current?.descriptor.key) tilesRef.current.markBuilt(p.level);
      }),
      onEvent('edt:tiles-failed', (p) => {
        if (p.key !== tilesRef.current?.descriptor.key) return;
        setBuildError(typeof p.error === 'string' ? p.error : p.error.message);
      }),
    ];
    return () =>
      subs.forEach(
        (s) =>
          void s.then(
            (u) => u(),
            () => undefined,
          ),
      );
  }, [desc]);

  // FR-EDT-13: the drawn stack follows the manifest; refreshed after every commit.
  useEffect(() => {
    setStack(project.layers);
    const sub = onEvent('project:committed', () => {
      api.project.get().then(
        (p) => p && p.project_id === project.project_id && setStack(p.layers),
        () => undefined,
      );
    });
    return () =>
      void sub.then(
        (u) => u(),
        () => undefined,
      );
  }, [project.project_id, project.layers]);

  useEffect(() => {
    if (!patchesLayer) return;
    const p = new PatchLayers(patchesLayer, project.project_id, (_id, msg) => setLayerError(msg));
    patchesRef.current = p;
    return () => {
      p.destroy();
      if (patchesRef.current === p) patchesRef.current = null;
    };
  }, [patchesLayer, project.project_id]);
  useEffect(() => {
    patchesRef.current?.setStack(stack);
  }, [stack, patchesLayer, project.project_id]);

  // FR-EDT-24: compare follows the store (set by the picker selection or RGN).
  useEffect(() => {
    if (!compareLayer) return;
    const c = new CompareLayer(compareLayer, setCompareError);
    compareRef.current = c;
    const s = editorStore.get();
    c.update(s.compare, s.compareMode, s.split, s.flipBefore);
    return () => {
      c.destroy();
      if (compareRef.current === c) compareRef.current = null;
    };
  }, [compareLayer]);
  useEffect(() => {
    compareRef.current?.update(compare, compareMode, split, flipBefore);
  }, [compare, compareMode, split, flipBefore]);
  useEffect(() => () => editorBridge.setCandidates(null), []);

  // Export's px-per-square values for Display size (FR-EDT-04, FR-EXP-26).
  useEffect(() => {
    let alive = true;
    api.export.getSettings().then(
      (s) =>
        alive &&
        editorStore.set({
          pxPerSquare: { source: s.source_px_per_square, target: s.target_px_per_square },
        }),
      () => undefined, // EXP not available yet → Display size stays disabled (R6).
    );
    const sub = onEvent('export:settings-changed', (p) =>
      editorStore.set({ pxPerSquare: { source: p.source, target: p.target } }),
    );
    return () => {
      alive = false;
      void sub.then(
        (u) => u(),
        () => undefined,
      );
    };
  }, [project.project_id]);

  // Tile manager lifetime follows the descriptor key and the tiles layer instance.
  const tilesBoundTo = useRef<Konva.Layer | null>(null);
  useEffect(() => {
    if (!tilesLayer) {
      // The Stage unmounted: its layer is gone, so drop the manager drawing into it.
      tilesRef.current?.destroy();
      tilesRef.current = null;
      tilesBoundTo.current = null;
      return;
    }
    if (!desc) return;
    if (tilesRef.current?.descriptor.key === desc.key && tilesBoundTo.current === tilesLayer)
      return;
    tilesRef.current?.destroy();
    const tm = new TileManager(konvaSink(tilesLayer), desc, setTileStats);
    tilesRef.current = tm;
    tilesBoundTo.current = tilesLayer;
    const s = editorStore.get();
    tm.update(s.view, s.viewport, s.dpr);
  }, [desc, tilesLayer]);
  useEffect(
    () => () => {
      tilesRef.current?.destroy();
      tilesRef.current = null;
    },
    [],
  );

  // FR-EDT-01: fit once the viewport is known (per project).
  const fitted = useRef<string | null>(null);
  useEffect(() => {
    if (fitted.current !== project.project_id && viewport.w > 0 && viewport.h > 0 && image) {
      fitted.current = project.project_id;
      editorStore.set({ view: fit(image, viewport) });
    }
  }, [project.project_id, viewport, image]);

  // Every view / viewport / DPR change re-plans tiles (FR-EDT-05, -08).
  useEffect(() => {
    tilesRef.current?.update(view, viewport, dpr);
  }, [view, viewport, dpr]);

  // FR-EDT-02: wheel zoom at the cursor (non-passive so the page doesn't scroll).
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const s = editorStore.get();
      if (!s.image) return;
      const r = el.getBoundingClientRect();
      const { min, max } = scaleLimits(s.image, s.viewport, s.dpr);
      const deltaY = e.deltaMode === 1 ? e.deltaY * 33.3 : e.deltaY;
      editorStore.set({
        view: zoomAbout(
          s.view,
          wheelFactor(deltaY),
          { x: e.clientX - r.left, y: e.clientY - r.top },
          min,
          max,
        ),
      });
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, []);

  // Shortcuts (FR-EDT-03/04/30). Space is held for temporary pan.
  useEffect(() => {
    const map: ShortcutMap = {
      'Ctrl+0': () => actions.fit(),
      'Ctrl+1': () => actions.actual(),
      'Ctrl+2': () => actions.display(),
      H: () => actions.setTool('pan'),
      R: () => actions.setTool('region'),
      B: () => actions.setTool('brush'),
      E: () => actions.setTool('eraser'),
      '[': () => actions.brushSize(-1),
      ']': () => actions.brushSize(1),
      'Shift+[': () => actions.hardness(-1),
      'Shift+]': () => actions.hardness(1),
      M: () => actions.toggleMask(),
      C: () =>
        editorStore.set((s) => ({ compareMode: s.compareMode === 'split' ? 'flip' : 'split' })),
    };
    const down = (e: KeyboardEvent) => {
      // FR-EDT-24 flip: "before" while the backslash key is held.
      if (e.code === 'Backslash' && !isTextTarget(e.target)) {
        e.preventDefault();
        if (!editorStore.get().flipBefore) editorStore.set({ flipBefore: true });
        return;
      }
      if (e.key === ' ' && !isTextTarget(e.target)) {
        e.preventDefault();
        setSpaceHeld(true);
        return;
      }
      dispatchShortcut(e, map);
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === ' ') setSpaceHeld(false);
      if (e.code === 'Backslash') editorStore.set({ flipBefore: false });
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
    };
  }, []);

  // FR-EDT-03: pan with the pan tool, Space+drag or middle-drag.
  const onPointerDown = (e: React.PointerEvent) => {
    // Overlay controls (badges, banners, handles with their own logic) aren't canvas input;
    // capturing the pointer here would steal their click.
    if ((e.target as Element).closest('button, a, input, [data-overlay]')) return;
    // Take focus from text fields, so tool shortcuts work after clicking the canvas (FR-EDT-30).
    containerRef.current?.focus({ preventScroll: true });
    const panning = e.button === 1 || (e.button === 0 && (tool === 'pan' || spaceHeld));
    if (!panning) {
      if (tools.onPointerDown(e)) {
        e.preventDefault();
        containerRef.current?.setPointerCapture?.(e.pointerId);
      }
      return;
    }
    e.preventDefault();
    drag.current = { kind: 'pan', x: e.clientX, y: e.clientY, pointerId: e.pointerId };
    containerRef.current?.setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    tools.onPointerMove(e);
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    d.x = e.clientX;
    d.y = e.clientY;
    const s = editorStore.get();
    if (s.image) editorStore.set({ view: panBy(s.view, dx, dy, s.image, s.viewport) });
  };
  const onPointerUp = (e: React.PointerEvent) => {
    tools.onPointerUp(e);
    if (drag.current?.pointerId === e.pointerId) drag.current = null;
  };

  useTestHook({
    store: editorStore.get,
    stage: () => stageRef.current,
    tiles: () => tilesRef.current?.stats() ?? null,
    extra: () => ({
      layers: () => patchesRef.current?.drawn() ?? [],
      stack: () => stack.map((l) => l.id),
      ...tools.testExtras(),
      setView: (v: { scale: number; tx: number; ty: number }) => editorStore.set({ view: v }),
      compareClip: () => compareRef.current?.clip() ?? null,
      compareLoaded: () => compareRef.current?.loaded() ?? false,
      /** Shows mocked candidates; handler calls are recorded in window.__pfPickerEvents. */
      setCandidates: (cands: Parameters<typeof editorBridge.setCandidates>[0]) => {
        const w = window as unknown as { __pfPickerEvents: [string, ...unknown[]][] };
        w.__pfPickerEvents = [];
        const rec =
          (name: string) =>
          (...a: unknown[]) =>
            w.__pfPickerEvents.push([name, ...a]);
        editorBridge.setCandidates(cands, {
          onSelect: rec('select'),
          onAccept: rec('accept'),
          onStar: rec('star'),
          onDiscard: rec('discard'),
        });
      },
    }),
  });

  const loaded = tileStats?.backdropShown ?? false;
  const panCursor = tool === 'pan' || spaceHeld;
  return (
    <div className="editor">
      <div
        ref={containerRef}
        className={`editor-canvas${panCursor ? ' editor-canvas-pan' : ''}`}
        data-testid="edt-canvas-stage"
        tabIndex={-1}
        data-image-loaded={loaded ? 'true' : 'false'}
        data-level={tileStats?.level ?? ''}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onAuxClick={(e) => e.preventDefault()}
      >
        {viewport.w > 0 && viewport.h > 0 && (
          <Stage
            ref={stageRef}
            width={viewport.w}
            height={viewport.h}
            x={view.tx}
            y={view.ty}
            scaleX={view.scale}
            scaleY={view.scale}
            listening={false}
          >
            <Layer
              ref={setTilesLayer}
              name="tiles"
              listening={false}
              imageSmoothingEnabled={view.scale * dpr < 1}
            />
            <Layer ref={setPatchesLayer} name="patches" listening={false} />
            <Layer
              ref={setMaskLayer}
              name="mask"
              listening={false}
              visible={maskVisible}
              imageSmoothingEnabled={false}
            />
            <Layer ref={setCompareLayer} name="compare" listening={false} />
          </Stage>
        )}
        {tools.overlay}
        {compare && compareMode === 'split' && (
          <CompareDivider view={view} src={compare} split={split} container={containerRef} />
        )}
        {compareError && (
          <div className="banner banner-error editor-banner" data-testid="edt-compare-error">
            The candidate could not be shown: {compareError}
          </div>
        )}
        {!loaded && !buildError && (
          <div className="editor-spinner" data-testid="edt-loading" aria-label="Loading" />
        )}
        {buildError && (
          <div className="banner banner-error editor-banner" data-testid="edt-build-error">
            The image could not be prepared: {buildError}{' '}
            <button type="button" className="link" onClick={() => void openTiles(true)}>
              Rebuild
            </button>
          </div>
        )}
        {layerError && (
          <div className="banner banner-error editor-banner" data-testid="edt-layer-error">
            A layer could not be shown: {layerError}
          </div>
        )}
        {!!tileStats?.failed && (
          <div className="editor-badge" data-testid="edt-tiles-error" role="status">
            Some tiles failed to load.{' '}
            <button
              type="button"
              className="link"
              data-testid="edt-tiles-rebuild"
              onClick={() => void openTiles(true)}
            >
              Rebuild
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
