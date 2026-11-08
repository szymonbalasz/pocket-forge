import { useProjectState } from '../../store/project';
import { EditorCanvas } from './EditorCanvas';
import './editor.css';

/** The open asset project's canvas. With none open, PRJ's home grid fills the slot (#45). */
export function EditorPanel() {
  const { project } = useProjectState();
  if (!project) return null;
  // Remount per project, so the view refits and the tiles reload.
  return <EditorCanvas key={project.project_id} project={project} />;
}
