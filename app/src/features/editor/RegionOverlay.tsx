// DOM overlay above the stage: the validated region (solid inner rect, dashed context margin,
// FR-EDT-27), the drag preview, the 8 resize handles, and the brush cursor (FR-EDT-17).
import { handlePoints } from './region';
import type { Rect, RegionState, Tool } from './store';
import { imageToScreen, type Pt, type View } from './view';

function boxStyle(r: Rect, v: View): React.CSSProperties {
  const a = imageToScreen(v, { x: r.x, y: r.y });
  return { left: a.x, top: a.y, width: r.w * v.scale, height: r.h * v.scale };
}

export function RegionOverlay({
  view,
  region,
  preview,
  tool,
  cursor,
  brushSize,
  hardness,
  onLeave,
  regionError,
  maskWarning,
}: {
  view: View;
  region: RegionState | null;
  preview: Rect | null;
  tool: Tool;
  cursor: Pt | null;
  brushSize: number;
  hardness: number;
  onLeave: () => void;
  regionError: { code: string; message: string } | null;
  maskWarning: string | null;
}) {
  const painting = tool === 'brush' || tool === 'eraser';
  const r = (brushSize / 2) * view.scale;
  return (
    <div className="edt-overlay" onPointerLeave={onLeave}>
      {region && (
        <>
          <div
            className="edt-region-margin"
            data-testid="edt-region-margin"
            style={boxStyle(region.outer, view)}
          />
          <div
            className="edt-region-box"
            data-testid="edt-region-box"
            data-rect={`${region.inner.x},${region.inner.y},${region.inner.w},${region.inner.h}`}
            style={boxStyle(region.inner, view)}
          />
          {tool === 'region' &&
            !preview &&
            Object.entries(handlePoints(region.inner, view)).map(([h, p]) => (
              <div
                key={h}
                className={`edt-region-handle edt-handle-${h}`}
                data-testid="edt-region-handle"
                data-handle={h}
                style={{ left: p.x, top: p.y }}
              />
            ))}
        </>
      )}
      {preview && (
        <div
          className="edt-region-box edt-region-preview"
          data-testid="edt-region-preview"
          style={boxStyle(preview, view)}
        />
      )}
      {painting && cursor && (
        <>
          <div
            className={`edt-brush${tool === 'eraser' ? ' edt-brush-erase' : ''}`}
            data-testid="edt-brush-cursor"
            data-radius={r}
            style={{ left: cursor.x - r, top: cursor.y - r, width: 2 * r, height: 2 * r }}
          />
          <div
            className="edt-brush edt-brush-hard"
            data-testid="edt-brush-cursor-hard"
            data-radius={r * hardness}
            style={{
              left: cursor.x - r * hardness,
              top: cursor.y - r * hardness,
              width: 2 * r * hardness,
              height: 2 * r * hardness,
            }}
          />
          {!region && (
            <div
              className="edt-cursor-hint"
              data-testid="edt-cursor-hint"
              style={{ left: cursor.x + 14, top: cursor.y + 14 }}
            >
              Select a region first
            </div>
          )}
        </>
      )}
      {regionError && (
        <div
          className="banner banner-warning editor-banner"
          data-testid="edt-region-error"
          data-code={regionError.code}
        >
          {regionError.message}
        </div>
      )}
      {maskWarning && (
        <div className="banner banner-warning editor-banner" data-testid="edt-mask-warning">
          {maskWarning}
        </div>
      )}
    </div>
  );
}
