// The PRJ status-bar segment (`prj-status-save`, asset-project §7): the open project's save
// state. Ctrl+S saves a revision (FR-PRJ-23).
import { useEffect } from 'react';

import { projectStore, useProjectState } from '../../store/project';
import { saveLabel, saveState } from './saveLabel';

export function ProjectStatus() {
  const { project, save } = useProjectState();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 's') {
        e.preventDefault();
        if (projectStore.get().project && !projectStore.get().project?.read_only)
          void projectStore.save();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  if (!project) return null;
  const state = saveState(save, project.read_only);
  const label = saveLabel(save, project.read_only);
  return (
    <span
      className={`prj-status prj-status-${state}`}
      data-testid="prj-status-save"
      data-state={state}
      title={project.project_dir}
    >
      <span className="prj-status-name">{project.name}</span>
      {/* BL-38: may ellipsise in a narrow bar; the full text stays readable to AT. */}
      <span className="prj-status-label" aria-label={label}>
        {label}
      </span>
    </span>
  );
}
