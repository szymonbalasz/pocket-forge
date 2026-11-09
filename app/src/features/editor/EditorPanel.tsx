import { useEffect } from 'react';

import { projectHasImage } from '../../shared/generateTabs';
import { useProjectState } from '../../store/project';
import { bindActions } from './actions';
import { EditorCanvas } from './EditorCanvas';
import { PreviewAlone } from './PreviewAlone';
import { undoStack } from './session';
import { dispatchShortcut, type ShortcutMap } from './shortcuts';
import { editorStore, useEditor } from './store';
import './editor.css';

/** The open asset project's canvas. With none open, PRJ's home grid fills the slot (#45). */
export function EditorPanel() {
  const { project } = useProjectState();
  useSessionUndo(project?.project_id ?? null);
  if (!project) return null;
  // FR-GEN-03: a project made by Generate new has nothing to draw until its first pick.
  if (!projectHasImage(project)) return <NoImage />;
  // Remount per project (and per bottom image, so a pick or its undo reloads the tiles).
  return (
    <EditorCanvas
      key={`${project.project_id}:${project.original_image_id ?? ''}`}
      project={project}
    />
  );
}

/** FR-EDT-21..23 / FR-GEN-29: the session's undo stack belongs to the open project, not to one
 *  canvas: a pick (or its undo) remounts the canvas for the new bottom image, or shows "No image
 *  yet", and the pick's entry must still undo and redo. The stack is cleared when the project
 *  changes or closes (KQ-16); Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y and the toolbar's undo summary are
 *  bound here. */
function useSessionUndo(projectId: string | null): void {
  useEffect(() => {
    if (!projectId) return;
    const off = undoStack.subscribe((s) =>
      editorStore.set({
        undo: { depth: s.depth, redo: s.redo, label: s.label, busy: s.busy, error: s.error },
      }),
    );
    const unbind = bindActions({
      undo: () => void undoStack.undo(),
      redo: () => void undoStack.redo(),
    });
    const map: ShortcutMap = {
      'Ctrl+Z': () => void undoStack.undo(),
      'Ctrl+Shift+Z': () => void undoStack.redo(),
      'Ctrl+Y': () => void undoStack.redo(),
    };
    const down = (e: KeyboardEvent) => void dispatchShortcut(e, map);
    window.addEventListener('keydown', down);
    return () => {
      window.removeEventListener('keydown', down);
      off();
      unbind();
      undoStack.clear();
    };
  }, [projectId]);
}

/** FR-GEN-03: the empty-project canvas state. A selected generate candidate shows here alone
 *  (FR-GEN-26: no working image to compare with). */
function NoImage() {
  const compare = useEditor((s) => s.compare);
  return (
    <div className="editor">
      <div className="editor-canvas editor-no-image" data-testid="edt-canvas-no-image">
        {compare ? (
          <PreviewAlone url={compare.url} />
        ) : (
          <p className="muted" role="status">
            No image yet. Generate one in New image.
          </p>
        )}
      </div>
    </div>
  );
}
