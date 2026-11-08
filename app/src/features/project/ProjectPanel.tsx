// The left project panel (asset-project §7, ui-guidelines): the open project's folders (Working,
// References, Structure, Parts, Results), then History (revisions) and Storage (disk use, trash).
// User-owned folders list what's on disk on open, on window focus and on Refresh (FR-PRJ-48).
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { Popover } from 'radix-ui';

import {
  api,
  pickPaths,
  type DiskUsage,
  type FolderFile,
  type RgnIteration,
  type ProjectFolder,
  type ProjectSnapshot,
  type RevisionEntry,
  type TrashItem,
} from '../../api';
import { IconPlus } from '../../shell/icons';
import { errorOf, projectStore, useProjectState } from '../../store/project';
import { ConflictView } from './ConflictView';
import { bytesLabel, CATEGORY_LABEL, dateLabel, revisionLabel, timeLabel } from './format';
import './project.css';

const USER_FOLDERS: { id: ProjectFolder; label: string }[] = [
  { id: 'references', label: 'References' },
  { id: 'structure', label: 'Structure' },
  { id: 'parts', label: 'Parts' },
];

export function ProjectPanel() {
  const { project } = useProjectState();
  if (!project) {
    return (
      <div className="prj-panel prj-panel-home" data-testid="prj-panel-home">
        <p className="muted small">Open a project to see its folders, history and storage.</p>
      </div>
    );
  }
  // Remount per project, so every list reloads.
  return <OpenProject key={project.project_id} project={project} />;
}

/** Re-runs `load` on window focus (FR-PRJ-48: files dropped in Explorer show up). */
function useOnFocus(load: () => void): void {
  useEffect(() => {
    window.addEventListener('focus', load);
    return () => window.removeEventListener('focus', load);
  }, [load]);
}

function OpenProject({ project }: { project: ProjectSnapshot }) {
  const { revision, error } = useProjectState();
  const readOnly = project.read_only !== null;
  return (
    <div className="prj-panel" data-testid="prj-panel" data-kind={project.kind}>
      <header className="prj-panel-header">
        <button
          type="button"
          className="link"
          data-testid="prj-panel-close"
          title="Close the project and go back to the home grid"
          onClick={() => void projectStore.close()}
        >
          ← Home
        </button>
        <h2 className="prj-panel-name" data-testid="prj-panel-name" title={project.project_dir}>
          {project.name}
        </h2>
      </header>
      {error && (
        <div className="banner banner-error" data-testid="prj-error" data-code={error.code}>
          {error.message}{' '}
          {error.code === 'unsaved' && (
            // FR-PRJ-21: closing with a save still pending asks first.
            <button
              type="button"
              className="link"
              data-testid="prj-panel-close-anyway"
              onClick={() => void projectStore.close(true)}
            >
              Close anyway
            </button>
          )}{' '}
          <button type="button" className="link" onClick={() => projectStore.clearError()}>
            Dismiss
          </button>
        </div>
      )}
      <Warnings project={project} />
      <Folder id="working" label="Working" revision={revision} canAdd={false} />
      {USER_FOLDERS.map((f) => (
        <Folder key={f.id} id={f.id} label={f.label} revision={revision} canAdd={!readOnly} />
      ))}
      <Results revision={revision} />
      <History project={project} revision={revision} />
      <Storage revision={revision} readOnly={readOnly} />
    </div>
  );
}

function Section({
  id,
  label,
  count,
  actions,
  children,
}: {
  id: string;
  label: string;
  count?: number;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <details className="prj-section" data-testid={`prj-panel-${id}`} open>
      <summary className="prj-section-head">
        <span>{label}</span>
        {count !== undefined && (
          <span className="prj-count tabular" data-testid={`prj-panel-${id}-count`}>
            {count}
          </span>
        )}
        <span className="pf-spacer" />
        {/* Buttons in a <summary> would toggle it: stop their clicks there. Clicks from portals
            (the Add… popover) bubble through React here too and must keep their default. */}
        <span
          className="prj-section-actions"
          onClick={(e) => {
            if (e.currentTarget.contains(e.target as Node)) e.preventDefault();
          }}
        >
          {actions}
        </span>
      </summary>
      <div className="prj-section-body">{children}</div>
    </details>
  );
}

// ------------------------------------------------------------------ warnings and relink

const WARNING_TEXT: Record<string, string> = {
  original_changed: 'Original changed: it was saved over by another editor. Read-only.',
  original_missing: 'Original missing. Relink it to keep editing.',
  newer_version: 'Made by a newer Pocket Forge. Read-only.',
  recovered: 'The project file was rebuilt from its revisions.',
  conflict: 'This project was also changed on another machine.',
  folder_moved:
    'This project’s folder was moved or renamed outside Pocket Forge, so changes aren’t saved. Reopen it from home.',
};

function Warnings({ project }: { project: ProjectSnapshot }) {
  const { save } = useProjectState();
  const [comparing, setComparing] = useState(false);
  const codes = new Set(project.warnings.map((w) => w.code));
  if (project.read_only) codes.add(project.read_only);
  if (save.kind === 'conflict') codes.add('conflict');
  if (save.kind === 'failed' && save.code === 'folder_moved') codes.add('folder_moved');
  if (codes.size === 0) return null;
  return (
    <div className="prj-warnings">
      {[...codes].map((code) => {
        const w = project.warnings.find((x) => x.code === code);
        return (
          <div
            key={code}
            className="banner banner-warning"
            data-testid="prj-panel-warning"
            data-code={code}
            role="status"
          >
            {WARNING_TEXT[code] ?? w?.message ?? code}
            {code === 'original_missing' && <Relink />}
            {code === 'folder_moved' && (
              // §11: closing asks first when the draft couldn't be saved (FR-PRJ-21).
              <button
                type="button"
                className="link"
                data-testid="prj-panel-reopen-home"
                onClick={() => void projectStore.close()}
              >
                Back to home
              </button>
            )}
            {code === 'conflict' && (
              <button
                type="button"
                className="link"
                data-testid="prj-panel-conflict-open"
                onClick={() => setComparing(true)}
              >
                Compare versions
              </button>
            )}
          </div>
        );
      })}
      <ConflictView open={comparing} onClose={() => setComparing(false)} />
    </div>
  );
}

/** FR-PRJ-16: accepts a file only if its hash matches; it is copied back into `working/`. */
function Relink() {
  const [path, setPath] = useState('');
  const [error, setError] = useState<string | null>(null);
  return (
    <form
      className="prj-relink"
      onSubmit={(e) => {
        e.preventDefault();
        setError(null);
        api.project.relinkOriginal(path.trim()).then(
          () => void projectStore.reload(),
          (err: unknown) => {
            const { code, message } = errorOf(err);
            setError(code === 'hash_mismatch' ? 'That file is not the original.' : message);
          },
        );
      }}
    >
      <input
        data-testid="prj-panel-relink-path"
        aria-label="Path of the original"
        placeholder="Path of the original"
        value={path}
        onChange={(e) => setPath(e.target.value)}
      />
      <button type="submit" data-testid="prj-panel-relink" disabled={!path.trim()}>
        Relink
      </button>
      {error && (
        <span className="error small" data-testid="prj-panel-relink-error" role="alert">
          {error}
        </span>
      )}
    </form>
  );
}

// ------------------------------------------------------------------ folders

function Folder({
  id,
  label,
  revision,
  canAdd,
}: {
  id: ProjectFolder;
  label: string;
  revision: number;
  canAdd: boolean;
}) {
  const [files, setFiles] = useState<FolderFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api.project.folderList(id).then(
      (f) => {
        setFiles(f);
        setError(null);
      },
      (e: unknown) => setError(errorOf(e).message),
    );
  }, [id]);
  useEffect(load, [load, revision]);
  useOnFocus(load);

  const add = async (paths: string[]) => {
    try {
      setFiles(await api.project.folderAdd(id, paths));
      setError(null);
    } catch (e) {
      setError(errorOf(e).message);
    }
  };

  return (
    <Section
      id={id}
      label={label}
      count={files?.length}
      actions={
        <>
          {canAdd && <AddFiles folder={id} onAdd={add} />}
          <button
            type="button"
            className="prj-icon-button"
            data-testid={`prj-panel-${id}-refresh`}
            title={`Refresh ${label}`}
            aria-label={`Refresh ${label}`}
            onClick={load}
          >
            ⟳
          </button>
        </>
      }
    >
      {error && (
        <p className="error small" data-testid={`prj-panel-${id}-error`}>
          {error}
        </p>
      )}
      {files && files.length === 0 && (
        <p className="muted small">
          {id === 'working' ? 'No files' : 'Empty. Add files here or drop them in the folder.'}
        </p>
      )}
      {files && files.length > 0 && (
        <ul className="prj-files">
          {files.map((f) => (
            <li
              key={f.name}
              className="prj-file"
              data-testid={`prj-panel-${id}-file`}
              data-record-id={f.name}
              data-image={f.image ? 'true' : undefined}
              title={`${f.name} · ${bytesLabel(f.bytes)}`}
            >
              {f.image ? (
                f.thumb_url ? (
                  <img className="prj-file-thumb" src={f.thumb_url} alt="" loading="lazy" />
                ) : (
                  <span className="prj-file-thumb prj-file-thumb-empty" aria-hidden="true" />
                )
              ) : (
                <span className="prj-file-icon" aria-hidden="true">
                  ·
                </span>
              )}
              <span className="prj-file-name">{f.name}</span>
              <span className="prj-file-size tabular">{bytesLabel(f.bytes)}</span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

/** Add… copies files into a user-owned folder (`project_folder_add`) from the native multi-select
 *  dialog (#53). Where no dialog is available (tests on the mocked core), a popover takes paths. */
function AddFiles({
  folder,
  onAdd,
}: {
  folder: ProjectFolder;
  onAdd: (paths: string[]) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState('');
  const paths = text
    .split(/\r?\n/)
    .map((l) => l.trim().replace(/^"(.*)"$/, '$1'))
    .filter(Boolean);
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Anchor asChild>
        <button
          type="button"
          className="prj-icon-button"
          data-testid={`prj-panel-${folder}-add`}
          title="Add… (copies files in)"
          aria-label={`Add files to ${folder}`}
          aria-haspopup="dialog"
          onClick={() =>
            pickPaths({ title: `Add files to ${folder}`, multiple: true }).then(
              (picked) => {
                if (picked.length) void onAdd(picked);
              },
              () => setOpen(true),
            )
          }
        >
          <IconPlus />
        </button>
      </Popover.Anchor>
      <Popover.Portal>
        <Popover.Content className="prj-add-popover" side="right" align="start" sideOffset={6}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              setOpen(false);
              setText('');
              void onAdd(paths);
            }}
          >
            <label className="small muted" htmlFor={`prj-add-${folder}`}>
              File paths to copy in, one per line
            </label>
            <textarea
              id={`prj-add-${folder}`}
              data-testid={`prj-panel-${folder}-add-paths`}
              rows={3}
              value={text}
              autoFocus
              onChange={(e) => setText(e.target.value)}
            />
            <div className="row">
              <span className="pf-spacer" />
              <button
                type="submit"
                className="primary"
                data-testid={`prj-panel-${folder}-add-confirm`}
                disabled={paths.length === 0}
              >
                Copy in
              </button>
            </div>
          </form>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

// ------------------------------------------------------------------ results

/** Iterations (`results/NNN <slug>/`), from RGN's list; RGN's tracker selects them. */
function Results({ revision }: { revision: number }) {
  const [items, setItems] = useState<RgnIteration[]>([]);
  useEffect(() => {
    // The active job before it has candidates has no number or folder yet: not a result.
    api.region.iterations().then(
      (rows) => setItems(rows.filter((r) => r.n !== null)),
      () => setItems([]),
    );
  }, [revision]);
  return (
    <Section id="results" label="Results" count={items.length}>
      {items.length === 0 ? (
        <p className="muted small">No iterations yet</p>
      ) : (
        <ul className="prj-files">
          {items.map((it) => (
            <li
              key={it.job_id}
              className="prj-file"
              data-testid="prj-panel-results-item"
              data-record-id={it.job_id}
              title={it.dir ?? undefined}
            >
              <span className="prj-file-name">
                {String(it.n).padStart(3, '0')} {it.slug ?? ''}
              </span>
              <span className="prj-file-size tabular">
                {it.kept} / {it.returned}
                {it.accepted_in_stack ? ' ✓' : ''}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

// ------------------------------------------------------------------ history

function History({ project, revision }: { project: ProjectSnapshot; revision: number }) {
  const [revs, setRevs] = useState<RevisionEntry[]>(project.revisions);
  const [label, setLabel] = useState('');
  const readOnly = project.read_only !== null;
  useEffect(() => {
    api.project.listRevisions().then(setRevs, () => undefined);
  }, [revision]);
  const newestFirst = [...revs].sort((a, b) => b.seq - a.seq);
  return (
    <Section id="history" label="History" count={revs.length}>
      <form
        className="prj-history-save"
        onSubmit={(e) => {
          e.preventDefault();
          void projectStore.save(label.trim() || undefined).then(() => setLabel(''));
        }}
      >
        <input
          data-testid="prj-panel-history-label"
          aria-label="Revision label"
          placeholder="Label (optional)"
          value={label}
          disabled={readOnly}
          onChange={(e) => setLabel(e.target.value)}
        />
        <button
          type="submit"
          data-testid="prj-panel-history-save"
          disabled={readOnly}
          title="Save a revision (Ctrl+S)"
        >
          Save
        </button>
      </form>
      <ul className="prj-files">
        {newestFirst.map((r) => (
          <li
            key={r.id}
            className="prj-file"
            data-testid="prj-panel-history-item"
            data-record-id={r.id}
            data-reason={r.reason}
            data-head={r.id === project.head_revision ? 'true' : undefined}
          >
            <span className="prj-rev-seq tabular">r{r.seq}</span>
            <span className="prj-file-name" title={r.created_at}>
              {revisionLabel(r)}
            </span>
            <span className="prj-file-size tabular">{timeLabel(r.created_at)}</span>
            <button
              type="button"
              className="link"
              data-testid="prj-panel-history-restore"
              disabled={readOnly}
              title={`Restore r${r.seq} as a new revision`}
              onClick={() => void projectStore.restore(r.id)}
            >
              Restore
            </button>
          </li>
        ))}
      </ul>
    </Section>
  );
}

// ------------------------------------------------------------------ storage

function Storage({ revision, readOnly }: { revision: number; readOnly: boolean }) {
  const [usage, setUsage] = useState<DiskUsage | null>(null);
  const [trash, setTrash] = useState<TrashItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const load = useCallback(() => {
    api.project.diskUsage().then(setUsage, (e: unknown) => {
      const err = errorOf(e);
      // `ipc`: the core doesn't register the command yet (PRJ slice 5): say so quietly.
      setError(err.code === 'ipc' ? null : err.message);
      setUnavailable(err.code === 'ipc');
    });
    api.project.trashList().then(setTrash, () => setTrash([]));
  }, []);
  useEffect(load, [load, revision]);

  const total = usage ? Object.values(usage.bytes).reduce((s, b) => s + b, 0) : 0;
  return (
    <Section id="storage" label="Storage">
      {error && <p className="error small">{error}</p>}
      {unavailable && (
        <p className="muted small" data-testid="prj-panel-storage-unavailable">
          Disk use and trash aren’t available yet.
        </p>
      )}
      {usage && (
        <>
          <dl className="prj-usage tabular" data-testid="prj-panel-storage-usage">
            {Object.entries(usage.bytes)
              .filter(([, b]) => b > 0)
              .map(([k, b]) => (
                <div key={k} data-category={k}>
                  <dt>{CATEGORY_LABEL[k] ?? k}</dt>
                  <dd>{bytesLabel(b)}</dd>
                </div>
              ))}
            <div className="prj-usage-total">
              <dt>Total</dt>
              <dd data-testid="prj-panel-storage-total">{bytesLabel(total)}</dd>
            </div>
          </dl>
          <p className="muted small" data-testid="prj-panel-storage-trash">
            Trash: {usage.trash_count} item{usage.trash_count === 1 ? '' : 's'},{' '}
            {bytesLabel(usage.bytes.trash)}
            {usage.next_purge ? ` · next purge ${dateLabel(usage.next_purge)}` : ''}
          </p>
        </>
      )}
      {trash.length > 0 && (
        <ul className="prj-files">
          {trash.map((t) => (
            <li
              key={t.id}
              className="prj-file"
              data-testid="prj-panel-storage-item"
              data-record-id={t.id}
              title={t.path}
            >
              <span className="prj-file-name">{t.path.split('/').pop()}</span>
              <span className="prj-file-size tabular">{bytesLabel(t.bytes)}</span>
              {t.restorable && (
                <button
                  type="button"
                  className="link"
                  data-testid="prj-panel-storage-restore"
                  disabled={readOnly}
                  onClick={() =>
                    api.project
                      .trashRestore(t.id)
                      .then(load, (e: unknown) => setError(errorOf(e).message))
                  }
                >
                  Restore
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}
