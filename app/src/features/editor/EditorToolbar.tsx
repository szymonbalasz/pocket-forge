// The editor's vertical toolbar (toolbar slot, EDT §7): tools, brush settings, mask actions,
// undo/redo and mask visibility. Every button has a shortcut (ui-guidelines).
// Fill, clear, undo and redo act through `actions` (mask and undo: slice 4).
import type { ComponentType } from 'react';

import {
  IconBrush,
  IconClear,
  IconEraser,
  IconFill,
  IconMask,
  IconPan,
  IconRedo,
  IconRegion,
  IconUndo,
} from '../../shell/icons';
import { projectHasImage } from '../../shared/generateTabs';
import { useProjectState } from '../../store/project';
import { actions } from './actions';
import { noMouseFocus } from './shortcuts';
import { useEditor, type Tool } from './store';
import { Tip } from './Tip';

/** FR-GEN-03: the tools are there but disabled, with the reason `no_image`. */
function NoImageToolbar() {
  return (
    <div className="edt-toolbar" role="toolbar" aria-orientation="vertical" aria-label="Tools">
      {TOOLS.map(({ tool: t, label, Icon }) => (
        <Tip key={t} text={`${label}: ${NO_IMAGE_REASON}`}>
          <button
            type="button"
            className="edt-tool"
            data-testid={`edt-toolbar-${t}`}
            data-reason="no_image"
            aria-label={label}
            aria-pressed={false}
            disabled
          >
            <Icon />
          </button>
        </Tip>
      ))}
    </div>
  );
}

const TOOLS: { tool: Tool; label: string; key: string; Icon: ComponentType }[] = [
  { tool: 'pan', label: 'Pan', key: 'H', Icon: IconPan },
  { tool: 'region', label: 'Region', key: 'R', Icon: IconRegion },
  { tool: 'brush', label: 'Brush', key: 'B', Icon: IconBrush },
  { tool: 'eraser', label: 'Eraser', key: 'E', Icon: IconEraser },
];

/** Shown on the disabled tools of a project with no image (FR-GEN-03, reason `no_image`). */
export const NO_IMAGE_REASON = 'No image yet. Generate one in New image.';

export function EditorToolbar() {
  const { tool, brushSize, hardness, maskVisible, image, region, undo, originalMissing } =
    useEditor((s) => s);
  const { project } = useProjectState();
  if (project && !projectHasImage(project)) return <NoImageToolbar />;
  if (!image) return null;
  // BL-10: with the original missing there is nothing to edit.
  const canPaint = region !== null && !originalMissing;
  return (
    <div
      className="edt-toolbar"
      role="toolbar"
      onMouseDown={noMouseFocus}
      aria-orientation="vertical"
      aria-label="Tools"
    >
      {TOOLS.map(({ tool: t, label, key, Icon }) => (
        <Tip key={t} text={`${label} (${key})`}>
          <button
            type="button"
            className="edt-tool"
            data-testid={`edt-toolbar-${t}`}
            aria-pressed={tool === t}
            aria-label={label}
            disabled={originalMissing}
            onClick={() => actions.setTool(t)}
          >
            <Icon />
          </button>
        </Tip>
      ))}
      <hr />
      <Tip text="Brush size in image px ([ and ])">
        <span className="edt-value tabular" data-testid="edt-toolbar-size" data-value={brushSize}>
          {brushSize}
        </span>
      </Tip>
      <Tip text="Hardness (Shift+[ and Shift+])">
        <span
          className="edt-value tabular"
          data-testid="edt-toolbar-hardness"
          data-value={Math.round(hardness * 100)}
        >
          {Math.round(hardness * 100)}%
        </span>
      </Tip>
      <hr />
      {/* Owner 2026-11-08: FR-EDT-16's Fill region, named for what it does to the edit. */}
      <Tip text="Mask whole region">
        <button
          type="button"
          className="edt-tool"
          data-testid="edt-toolbar-mask-region"
          aria-label="Mask whole region"
          disabled={!canPaint}
          onClick={actions.fill}
        >
          <IconFill />
        </button>
      </Tip>
      <Tip text="Clear mask">
        <button
          type="button"
          className="edt-tool"
          data-testid="edt-toolbar-clear"
          aria-label="Clear mask"
          disabled={!canPaint}
          onClick={actions.clear}
        >
          <IconClear />
        </button>
      </Tip>
      <Tip
        text={
          undo.depth
            ? `Undo ${undo.label ?? ''} (Ctrl+Z) · ${undo.depth} step${undo.depth === 1 ? '' : 's'} left`
            : 'Nothing to undo (Ctrl+Z)'
        }
      >
        <button
          type="button"
          className="edt-tool"
          data-testid="edt-toolbar-undo"
          aria-label="Undo"
          data-depth={undo.depth}
          disabled={!undo.depth || undo.busy}
          onClick={actions.undo}
        >
          <IconUndo />
        </button>
      </Tip>
      <Tip text="Redo (Ctrl+Shift+Z)">
        <button
          type="button"
          className="edt-tool"
          data-testid="edt-toolbar-redo"
          aria-label="Redo"
          data-depth={undo.redo}
          disabled={!undo.redo || undo.busy}
          onClick={actions.redo}
        >
          <IconRedo />
        </button>
      </Tip>
      <hr />
      <Tip text="Show mask (M)">
        <button
          type="button"
          className="edt-tool"
          data-testid="edt-toolbar-mask"
          aria-label="Show mask"
          aria-pressed={maskVisible}
          onClick={actions.toggleMask}
        >
          <IconMask />
        </button>
      </Tip>
    </div>
  );
}
