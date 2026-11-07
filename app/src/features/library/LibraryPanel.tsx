// Library side panel: first-run root, import (drop or path), recent projects.
// Folder import, clipboard paste and progress are PRJ slice 2.
import { useCallback, useEffect, useState } from 'react';

import { api, onFileDrop, type ImportItem, type LibraryConfig, type RecentEntry } from '../../api';
import { projectStore, useProjectState } from '../../store/project';
import { summarise } from './summary';

export function LibraryPanel() {
  const { project, busy } = useProjectState();
  const [config, setConfig] = useState<LibraryConfig | null>(null);
  const [recent, setRecent] = useState<RecentEntry[]>([]);
  const [summary, setSummary] = useState<ImportItem[] | null>(null);
  const [rootInput, setRootInput] = useState('');
  const [rootError, setRootError] = useState<string | null>(null);
  const [importPath, setImportPath] = useState('');
  const [dragOver, setDragOver] = useState(false);

  const refresh = useCallback(async () => {
    const [c, r] = await Promise.all([api.library.getConfig(), api.recent.list()]);
    setConfig(c);
    setRecent(r);
  }, []);

  useEffect(() => {
    refresh().catch(() => undefined);
  }, [refresh, project?.project_id]);

  const runImport = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      setSummary(await projectStore.importAndOpen(paths));
      await refresh();
    },
    [refresh],
  );

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    onFileDrop((e) => {
      setDragOver(e.type === 'over');
      if (e.type === 'drop') void runImport(e.paths);
    })
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => undefined);
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [runImport]);

  const setRoot = async () => {
    setRootError(null);
    try {
      setConfig(await api.library.setRoots([rootInput.trim()]));
    } catch (e) {
      setRootError((e as Error).message);
    }
  };

  if (config && !config.configured) {
    return (
      <section className="panel" data-testid="prj-setup">
        <h2>Choose your library folder</h2>
        <p className="muted">Images and asset projects live here. A cloud-synced folder works.</p>
        <input
          data-testid="prj-setup-root-path"
          placeholder="Folder path"
          value={rootInput}
          onChange={(e) => setRootInput(e.target.value)}
        />
        <button
          type="button"
          data-testid="prj-setup-pick-root"
          onClick={setRoot}
          disabled={!rootInput.trim()}
        >
          Use this folder
        </button>
        {rootError && <p className="error">{rootError}</p>}
      </section>
    );
  }

  const unavailable = config?.roots.filter((r) => !r.available) ?? [];

  return (
    <section className="panel">
      {unavailable.length > 0 && (
        <div className="banner banner-error" data-testid="prj-library-unavailable">
          Library unavailable: {unavailable.map((r) => r.path).join(', ')}
        </div>
      )}
      <div
        className={`dropzone${dragOver ? ' dropzone-over' : ''}`}
        data-testid="prj-home-dropzone"
      >
        Drop images here to import them
      </div>
      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          void runImport(
            importPath
              .split('\n')
              .map((p) => p.trim())
              .filter(Boolean),
          );
        }}
      >
        <textarea
          data-testid="prj-home-import-path"
          placeholder="…or paste image paths, one per line"
          rows={2}
          value={importPath}
          onChange={(e) => setImportPath(e.target.value)}
        />
        <button type="submit" data-testid="prj-home-import" disabled={busy || !importPath.trim()}>
          Import
        </button>
      </form>
      {summary && (
        <p className="muted" data-testid="prj-import-summary">
          {summarise(summary)}
        </p>
      )}
      <h2>Recent projects</h2>
      <ul className="recent" data-testid="prj-home-recent">
        {recent.length === 0 && <li className="muted">Nothing yet</li>}
        {recent.map((r) => (
          <li key={r.path}>
            <button
              type="button"
              data-testid="prj-home-recent-item"
              data-record-id={r.path}
              className={project?.image_path === r.path ? 'active' : undefined}
              title={r.path}
              onClick={() => void projectStore.open(r.path)}
            >
              {r.name}
            </button>
          </li>
        ))}
      </ul>
      {config?.overridden && (
        <p className="muted small">Library: {config.roots[0]?.path} (PF_LIBRARY_DIR)</p>
      )}
    </section>
  );
}
