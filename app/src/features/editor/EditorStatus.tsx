// Status-bar part of the editor (FR-EDT-04): zoom readout and the Fit / Actual / Display presets.
import { actions } from './actions';
import { useEditor } from './store';
import { Tip } from './Tip';
import { displayScale, zoomPercent } from './view';

export function EditorStatus() {
  const { view, dpr, image, pxPerSquare } = useEditor((s) => s);
  if (!image) return null;
  const display = displayScale(pxPerSquare.source, pxPerSquare.target);
  return (
    <div className="edt-status">
      <Tip text="Fit (Ctrl+0)" side="top">
        <button type="button" data-testid="edt-view-fit" onClick={actions.fit}>
          Fit
        </button>
      </Tip>
      <Tip text="Actual pixels: 1 image px = 1 screen px (Ctrl+1)" side="top">
        <button type="button" data-testid="edt-view-actual" onClick={actions.actual}>
          1:1
        </button>
      </Tip>
      <Tip
        side="top"
        text={
          display === null
            ? 'Display size needs both px-per-square values in Export'
            : `Display size: ${pxPerSquare.target} px per square in the VTT (Ctrl+2)`
        }
      >
        <button
          type="button"
          data-testid="edt-view-display"
          disabled={display === null}
          onClick={actions.display}
        >
          Display
        </button>
      </Tip>
      <span data-testid="edt-view-zoom" className="tabular edt-zoom" title="Image px : device px">
        {zoomPercent(view, dpr)} %
      </span>
      <span className="tabular muted">
        {image.w} × {image.h} px
      </span>
    </div>
  );
}
