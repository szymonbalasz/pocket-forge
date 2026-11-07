import { useProjectState } from '../../store/project';
import { EditorCanvas } from './EditorCanvas';
import './editor.css';

/** The editor area: the empty state, or the open asset project's canvas. */
export function EditorPanel() {
  const { project } = useProjectState();
  if (!project) {
    return (
      <div className="editor-empty" data-testid="edt-empty">
        <p>Open an image</p>
        <p className="muted">Drop an image on the window, or pick one from Recent projects.</p>
      </div>
    );
  }
  // Remount per project, so the view refits and the tiles reload.
  return <EditorCanvas key={project.project_id} project={project} />;
}
