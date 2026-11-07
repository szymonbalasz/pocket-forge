// Minimal editor canvas: the original on a Konva stage with wheel zoom and drag pan.
// Tiles, layers, masks and tools are EDT slices 1–6.
import type Konva from 'konva';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Image as KonvaImage, Layer, Stage } from 'react-konva';

import { api, type ProjectSnapshot } from '../../api';
import { fit, panBy, scaleLimits, wheelFactor, zoomAbout, zoomPercent, type View } from './view';

declare global {
  interface Window {
    /** Test hook (FR-EDT-26): dev and test builds only. */
    __pfTest?: {
      view: () => View;
      imageSize: () => { w: number; h: number } | null;
      imageLoaded: () => boolean;
      stage: () => Konva.Stage | null;
    };
  }
}

function useElementSize(ref: React.RefObject<HTMLElement | null>): { w: number; h: number } {
  const [size, setSize] = useState({ w: 0, h: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return size;
}

export function EditorStage({ project }: { project: ProjectSnapshot }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<Konva.Stage>(null);
  const viewport = useElementSize(containerRef);
  const image = { w: project.width, h: project.height };
  const [img, setImg] = useState<HTMLImageElement | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [view, setView] = useState<View>({ scale: 1, tx: 0, ty: 0 });
  const viewRef = useRef(view);
  viewRef.current = view;
  const fitted = useRef(false);
  const dpr = window.devicePixelRatio || 1;

  const originalId = project.layers[0]?.id;
  useEffect(() => {
    if (!originalId) return;
    let alive = true;
    setImg(null);
    setLoadError(null);
    fitted.current = false;
    api.editor
      .imageOpen(project.project_id, originalId)
      .then(({ url }) => {
        const el = new window.Image();
        el.crossOrigin = 'anonymous';
        el.onload = () => alive && setImg(el);
        el.onerror = () => alive && setLoadError('The image could not be loaded.');
        el.src = url;
      })
      .catch((e: Error) => alive && setLoadError(e.message));
    return () => {
      alive = false;
    };
  }, [project.project_id, originalId]);

  const doFit = useCallback(() => {
    if (viewport.w > 0 && viewport.h > 0) setView(fit(image, viewport));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewport.w, viewport.h, image.w, image.h]);

  // FR-EDT-01: fit once the viewport is known.
  useEffect(() => {
    if (!fitted.current && viewport.w > 0) {
      fitted.current = true;
      doFit();
    }
  }, [viewport.w, doFit]);

  // Wheel zoom anchored at the cursor (non-passive so the page doesn't scroll).
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el.getBoundingClientRect();
      const { min, max } = scaleLimits(image, viewport, dpr);
      setView((v) =>
        zoomAbout(
          v,
          wheelFactor(e.deltaY),
          { x: e.clientX - r.left, y: e.clientY - r.top },
          min,
          max,
        ),
      );
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [image.w, image.h, viewport, dpr]); // eslint-disable-line react-hooks/exhaustive-deps

  // Drag to pan (left, middle or Space+drag all pan while the pan tool is the only tool).
  const drag = useRef<{ x: number; y: number } | null>(null);
  const onPointerDown = (e: React.PointerEvent) => {
    drag.current = { x: e.clientX, y: e.clientY };
    (e.target as Element).setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    drag.current = { x: e.clientX, y: e.clientY };
    setView((v) => panBy(v, dx, dy, image, viewport));
  };
  const onPointerUp = () => {
    drag.current = null;
  };

  const actual = () => {
    const c = { x: viewport.w / 2, y: viewport.h / 2 };
    setView((v) => zoomAbout(v, 1 / dpr / v.scale, c, 0, Infinity));
  };

  // FR-EDT-04 presets (display size needs EXP's settings; it lands with EDT slice 2).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        !(e.ctrlKey || e.metaKey) ||
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLTextAreaElement
      )
        return;
      if (e.key === '0') {
        e.preventDefault();
        doFit();
      } else if (e.key === '1') {
        e.preventDefault();
        actual();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  useEffect(() => {
    if (!import.meta.env.DEV) return;
    window.__pfTest = {
      view: () => viewRef.current,
      imageSize: () => image,
      imageLoaded: () => img !== null,
      stage: () => stageRef.current,
    };
    return () => {
      delete window.__pfTest;
    };
  });

  return (
    <div className="editor">
      <div
        ref={containerRef}
        className="editor-canvas"
        data-testid="edt-canvas-stage"
        data-image-loaded={img ? 'true' : 'false'}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onAuxClick={(e) => e.preventDefault()}
      >
        {viewport.w > 0 && (
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
            <Layer imageSmoothingEnabled={view.scale * dpr < 1}>
              {img && <KonvaImage image={img} x={0} y={0} width={image.w} height={image.h} />}
            </Layer>
          </Stage>
        )}
        {loadError && (
          <div className="banner banner-error" data-testid="edt-image-error">
            {loadError}
          </div>
        )}
      </div>
      <div className="editor-bar">
        <button type="button" data-testid="edt-view-fit" onClick={doFit} title="Fit (Ctrl+0)">
          Fit
        </button>
        <button
          type="button"
          data-testid="edt-view-actual"
          onClick={actual}
          title="Actual pixels (Ctrl+1)"
        >
          100 %
        </button>
        <span data-testid="edt-view-zoom" className="readout">
          {zoomPercent(view, dpr)} %
        </span>
        <span className="readout muted">
          {project.width} × {project.height} px
        </span>
      </div>
    </div>
  );
}
