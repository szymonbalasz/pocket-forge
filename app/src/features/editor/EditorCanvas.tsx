// The editor canvas (canvas slot): a Konva stage whose transform is the app-owned view
// (FR-EDT-06), the tile layer drawn by TileManager, wheel zoom, pan, presets and HiDPI.
import Konva from 'konva';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Layer, Stage } from 'react-konva';

import {
  api,
  ApiError,
  onEvent,
  type Layer as ManifestLayer,
  type ProjectSnapshot,
  type TilesDescriptor,
} from '../../api';
import { CanvasMenu } from '../export';
import { actions } from './actions';
import { useTestHook } from './hook';
import { dispatchShortcut, isControlKey, isTextTarget, type ShortcutMap } from './shortcuts';
import { editorStore, useEditor } from './store';
import { editorBridge } from './bridge';
import { CompareDivider } from './CompareDivider';
import { CompareLayer } from './CompareLayer';
import { PreviewAlone } from './PreviewAlone';
import { PatchLayers } from './PatchLayers';
import { useMaskTools } from './useMaskTools';
import { konvaSink, TileManager, type TileStats } from './TileManager';
import { TileEventLog } from './tileEvents';
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

/** A pyramid build failure, said plainly (D6). Rebuild is offered only where it can help. */
interface BuildError {
  code: string;
  message: string;
}

const BUILD_TEXT: Record<string, string> = {
  not_found: 'The image file can’t be found.',
  unsupported: 'This image format isn’t supported.',
  animated: 'Animated images aren’t supported.',
  too_large: 'The image is larger than 16384 px per side.',
  corrupt: 'The image file is damaged and can’t be read.',
};

export const buildErrorText = (e: BuildError): string =>
  BUILD_TEXT[e.code] ?? `The image could not be prepared: ${e.message}`;

/** Rebuild re-reads the same source, so it can't fix a missing, damaged or unsupported file. */
export const canRebuild = (e: BuildError): boolean => !(e.code in BUILD_TEXT);

function buildError(e: unknown): BuildError {
  if (e instanceof ApiError) return { code: e.code, message: e.message };
  if (e && typeof e === 'object' && 'code' in e && 'message' in e)
    return { code: String(e.code), message: String(e.message) };
  return { code: 'error', message: e instanceof Error ? e.message : String(e) };
}

/** The project the canvas last mounted for (a remount for a new bottom image keeps state). */
let lastCanvasProject: string | null = null;

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
  const [buildErr, setBuildError] = useState<BuildError | null>(null);
  const [spaceHeld, setSpaceHeld] = useState(false);
  const drag = useRef<Drag | null>(null);
  const { view, viewport, dpr, tool, image, maskVisible } = useEditor((s) => s);
  const { compare, compareMode, split, flipBefore } = useEditor((s) => s);
  const tools = useMaskTools({ project, maskLayer, container: containerRef });

  useElementSize(containerRef);
  useDevicePixelRatio(stageRef);

  // #48 (3): the original's stable image id (PRJ `resolve_image`), so a reopen reuses the pyramid.
  const originalId = project.original_image_id ?? project.layers[0]?.id;
  // BL-10 / D6: no original, nothing to show or edit until PRJ relinks it.
  const originalMissing = project.read_only === 'original_missing';

  // Tile events, subscribed before any `tilesOpen` (D1): a small pyramid is built and reported
  // before the call returns. Every event is recorded, so a descriptor or tile manager created
  // later still learns what was built or failed; the descriptor is the state at subscribe time.
  const tileLog = useRef(new TileEventLog());
  const openKey = useRef<string | null>(null);
  const subscribed = useRef<Promise<unknown>>(Promise.resolve());
  useEffect(() => {
    const subs = [
      onEvent('edt:tiles-progress', (p) => {
        tileLog.current.progress(p.key, p.level);
        if (p.key === tilesRef.current?.descriptor.key) tilesRef.current.markBuilt(p.level);
      }),
      onEvent('edt:tiles-failed', (p) => {
        const err = tileLog.current.fail(p.key, p.error);
        if (p.key === openKey.current) setBuildError(err);
      }),
    ];
    subscribed.current = Promise.allSettled(subs);
    return () =>
      subs.forEach(
        (s) =>
          void s.then(
            (u) => u(),
            () => undefined,
          ),
      );
  }, []);

  // Open the pyramid (FR-EDT-07): the descriptor comes back at once; levels arrive by event.
  const openTiles = useCallback(
    async (rebuild: boolean) => {
      if (!originalId) return;
      setBuildError(null);
      if (rebuild) tileLog.current.clear();
      try {
        await subscribed.current;
        const d = tileLog.current.apply(
          await api.editor.tilesOpen(project.project_id, originalId, rebuild),
        );
        openKey.current = d.key;
        setDesc(d);
        if (rebuild) tilesRef.current?.reset(d);
        const failed = tileLog.current.failure(d.key);
        if (failed) setBuildError(failed);
      } catch (e) {
        setBuildError(buildError(e));
      }
    },
    [project.project_id, originalId],
  );

  useEffect(() => {
    // A remount for a new bottom image of the same project (a pick or its undo, FR-GEN-29)
    // keeps the picker, the compare source and the undo summary (the panel owns undo).
    const same = lastCanvasProject === project.project_id;
    lastCanvasProject = project.project_id;
    editorStore.reset(same ? ['picker', 'compare', 'compareMode', 'split', 'undo'] : ['undo']);
    editorStore.set({ image: { w: project.width, h: project.height } });
    return () => editorStore.set({ image: null });
  }, [project.project_id, project.width, project.height]);

  // After a relink the original is back: open (or reopen, with a new key) its tiles.
  useEffect(() => {
    editorStore.set({ originalMissing });
    if (originalMissing) {
      setDesc(null);
      return;
    }
    void openTiles(false);
  }, [originalMissing, openTiles]);

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
    c.update(s.compare?.alone ? null : s.compare, s.compareMode, s.split, s.flipBefore);
    return () => {
      c.destroy();
      if (compareRef.current === c) compareRef.current = null;
    };
  }, [compareLayer]);
  useEffect(() => {
    // FR-GEN-26: a whole image of another size is shown alone (below), not split or flipped.
    compareRef.current?.update(compare?.alone ? null : compare, compareMode, split, flipBefore);
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
    const tm = new TileManager(konvaSink(tilesLayer), tileLog.current.apply(desc), setTileStats);
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
      // Space on a focused button presses it (#48); elsewhere it holds temporary pan.
      if (e.key === ' ' && !isControlKey(e)) {
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
    // A focus loss (Alt-Tab) sends the keyups elsewhere: release what was held (FR-EDT-03/24).
    const blur = () => {
      setSpaceHeld(false);
      if (editorStore.get().flipBefore) editorStore.set({ flipBefore: false });
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
      window.removeEventListener('blur', blur);
    };
  }, []);

  // FR-EDT-03: pan with the pan tool, Space+drag or middle-drag.
  const onPointerDown = (e: React.PointerEvent) => {
    // Overlay controls (badges, banners, handles with their own logic) aren't canvas input;
    // capturing the pointer here would steal their click.
    if ((e.target as Element).closest('button, a, input, [data-overlay]')) return;
    if (editorStore.get().originalMissing) return;
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
          // #56: like RGN, "Discard the rest" only once one is accepted.
          ...(cands?.some((c) => c.accepted) ? { onDiscardRest: rec('discard-rest') } : {}),
        });
      },
    }),
  });

  const loaded = (tileStats?.backdropShown ?? false) && !originalMissing;
  const panCursor = tool === 'pan' || spaceHeld;
  return (
    <div className="editor">
      {/* #63: right-click → Save as…, Export as ▸, Open in Explorer (EDT §7). */}
      <CanvasMenu>
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
          {/* FR-GEN-26: alone, fit to view; holding \ shows the working image. */}
          {compare?.alone && !flipBefore && <PreviewAlone url={compare.url} />}
          {compare && !compare.alone && compareMode === 'split' && (
            <CompareDivider view={view} src={compare} split={split} container={containerRef} />
          )}
          {compareError && (
            <div className="banner banner-error editor-banner" data-testid="edt-compare-error">
              The candidate could not be shown: {compareError}
            </div>
          )}
          {!loaded && !buildErr && !originalMissing && (
            <div className="editor-spinner" data-testid="edt-loading" aria-label="Loading" />
          )}
          {originalMissing && (
            <div
              className="banner banner-warning editor-banner"
              data-testid="edt-original-missing"
              role="status"
            >
              The original image is missing, so there is nothing to edit. Relink it in the project
              panel to keep working.
            </div>
          )}
          {buildErr && !originalMissing && (
            <div
              className="banner banner-error editor-banner"
              data-testid="edt-build-error"
              data-code={buildErr.code}
            >
              {buildErrorText(buildErr)}{' '}
              {canRebuild(buildErr) && (
                <button
                  type="button"
                  className="link"
                  data-testid="edt-build-rebuild"
                  onClick={() => void openTiles(true)}
                >
                  Rebuild
                </button>
              )}
            </div>
          )}
          {layerError && (
            <div className="banner banner-error editor-banner" data-testid="edt-layer-error">
              A layer could not be shown: {layerError}
            </div>
          )}
          {!!tileStats?.failed && !originalMissing && (
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
      </CanvasMenu>
    </div>
  );
}
