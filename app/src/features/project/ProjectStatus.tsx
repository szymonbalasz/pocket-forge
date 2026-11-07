// Status bar for the open asset project: name, save status, Save (Ctrl+S), Close.
import { useEffect } from 'react';

import { projectStore, useProjectState, type SaveStatus } from '../../store/project';
import { saveLabel } from './saveLabel';

export function ProjectStatus() {
  const { project, save, error } = useProjectState();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
        e.preventDefault();
        void projectStore.save();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <div className="status">
      {error && (
        <span className="error" data-testid="prj-error" data-code={error.code}>
          {error.message}{' '}
          <button type="button" className="link" onClick={() => projectStore.clearError()}>
            Dismiss
          </button>
        </span>
      )}
      {project && (
        <>
          <span className="status-name" title={project.image_path}>
            {project.name}
          </span>
          <span data-testid="prj-status-save" data-state={stateOf(save, project.read_only)}>
            {saveLabel(save, project.read_only)}
          </span>
          {project.warnings.map((w) => (
            <span
              key={w.code}
              className="warning"
              data-testid="prj-status-warning"
              data-code={w.code}
            >
              {w.message}
            </span>
          ))}
          <span className="spacer" />
          <button
            type="button"
            data-testid="prj-toolbar-save"
            disabled={project.read_only !== null || save.kind === 'saving'}
            onClick={() => void projectStore.save()}
            title="Save a revision (Ctrl+S)"
          >
            Save
          </button>
          <button
            type="button"
            data-testid="prj-toolbar-close"
            onClick={() => void projectStore.close()}
          >
            Close
          </button>
        </>
      )}
    </div>
  );
}

function stateOf(save: SaveStatus, readOnly: string | null): string {
  return readOnly ? 'read_only' : save.kind;
}
