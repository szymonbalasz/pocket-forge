import { projectHasImage } from '../../shared/generateTabs';
import { useProjectState } from '../../store/project';
import { EditorCanvas } from './EditorCanvas';
import './editor.css';

/** The open asset project's canvas. With none open, PRJ's home grid fills the slot (#45). */
export function EditorPanel() {
  const { project } = useProjectState();
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

/** FR-GEN-03: the empty-project canvas state. */
function NoImage() {
  return (
    <div className="editor">
      <div className="editor-canvas editor-no-image" data-testid="edt-canvas-no-image">
        <p className="muted" role="status">
          No image yet. Generate one in New image.
        </p>
      </div>
    </div>
  );
}
