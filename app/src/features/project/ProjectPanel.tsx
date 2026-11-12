// The left project panel (asset-project §7, ui-guidelines): the open project's folders (Working,
// References, Structure, Parts, Results), then History (revisions) and Storage (disk use, trash).
// User-owned folders list what's on disk on open, on window focus and on Refresh (FR-PRJ-48).
// Files dragged from Explorer onto Working, References, Structure or Parts are taken in there
// (FR-PRJ-55, #61); a drop anywhere else in an open project does nothing.
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Popover } from 'radix-ui';

import {
  api,
  onFileDrop,
  pickPaths,
  type CandidateRecord,
  type DropPoint,
  type DiskUsage,
  type FolderFile,
  type RgnIteration,
  type ProjectFolder,
  type ProjectSnapshot,
  type RevisionEntry,
  type TrashItem,
} from '../../api';
import { ConfirmAsk } from '../../shared/ConfirmAsk';
import { IconPlus } from '../../shell/icons';
import { DeleteProjectDialog } from '../library';
import { dropNewImage, PATCHES_KEPT, useActiveJobId } from '../region-edit';
import {
  errorOf,
  folderMoved,
  FOLDER_MOVED_TEXT,
  isFolderMovedError,
  projectStore,
  useProjectState,
} from '../../store/project';
import { ConflictView } from './ConflictView';
import {
  bytesLabel,
  dateLabel,
  DROP_ONE_IMAGE,
  dropName,
  folderDropNotice,
  revisionLabel,
  storageLabel,
  timeLabel,
  workingDropError,
  type DropSkip,
} from './format';
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

// ------------------------------------------------------------------ drops (FR-PRJ-55, #61)

/** The sections a file drag can land on. */
type DropSection = 'working' | 'references' | 'structure' | 'parts';

const DROP_ATTR = 'data-drop-section';

/** The drop-target section under a page point, or null (elsewhere: the drop does nothing). */
function sectionAt(pt: DropPoint | null): DropSection | null {
  if (!pt || typeof document.elementFromPoint !== 'function') return null;
  const zone = document.elementFromPoint(pt.x, pt.y)?.closest(`[${DROP_ATTR}]`);
  return (zone?.getAttribute(DROP_ATTR) as DropSection | null | undefined) ?? null;
}

interface DropNotice {
  at: DropSection;
  kind: 'info' | 'error';
  text: string;
}

type DropOutcome = Omit<DropNotice, 'at'>;

/** Codes that refuse the whole drop on a user folder (not one file of it). */
const WHOLE_DROP_ERRORS = new Set(['read_only', 'no_project', 'unknown_folder', 'ipc']);

/** References / Structure / Parts: each file is copied in through `project_folder_add`
 *  (FR-PRJ-47: its naming, dedupe and errors), one at a time so one bad file skips only itself. */
async function dropInFolder(
  folder: ProjectFolder,
  label: string,
  paths: string[],
): Promise<DropOutcome> {
  const added: string[] = [];
  const skipped: DropSkip[] = [];
  let refused: string | null = null;
  for (const path of paths) {
    try {
      await api.project.folderAdd(folder, [path]);
      added.push(dropName(path));
    } catch (e) {
      const { code, message } = errorOf(e);
      if (isFolderMovedError(e) || WHOLE_DROP_ERRORS.has(code)) {
        refused = workingDropError('', isFolderMovedError(e) ? 'folder_moved' : code, message);
        break;
      }
      skipped.push({ name: dropName(path), code });
    }
  }
  if (added.length) projectStore.folderChanged(folder);
  if (refused && !added.length) return { kind: 'error', text: refused };
  const text = folderDropNotice(label, added, skipped);
  return {
    kind: added.length ? 'info' : 'error',
    text: refused ? `${text} ${refused}` : text,
  };
}

/** Working: exactly one image becomes the bottom image, through the pick path (one undoable
 *  revision, refused while a job runs or on a read-only project). */
async function dropOnWorking(paths: string[]): Promise<DropOutcome> {
  if (paths.length !== 1) return { kind: 'error', text: DROP_ONE_IMAGE };
  const name = dropName(paths[0]!);
  const r = await dropNewImage(paths[0]!);
  if (!r.ok) return { kind: 'error', text: workingDropError(name, r.code, r.message) };
  projectStore.folderChanged('working');
  const text = `New image dropped: ${name}.`;
  return {
    kind: 'info',
    text: r.patchesDropped ? `${text} ${PATCHES_KEPT(r.patchesDropped)}` : text,
  };
}

const SECTION_LABEL: Record<DropSection, string> = {
  working: 'Working',
  references: 'References',
  structure: 'Structure',
  parts: 'Parts',
};

/** Listens to native file drags while the project is open: whether one is over the window,
 *  which section is under the pointer, and what the last drop did. */
function usePanelDrop(readOnly: boolean): {
  dragging: boolean;
  over: DropSection | null;
  notice: DropNotice | null;
} {
  const [dragging, setDragging] = useState(false);
  const [over, setOver] = useState<DropSection | null>(null);
  const [notice, setNotice] = useState<DropNotice | null>(null);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    onFileDrop((e) => {
      if (e.type === 'over') {
        setDragging(true);
        setOver(sectionAt(e.position));
        return;
      }
      setDragging(false);
      setOver(null);
      if (e.type !== 'drop') return;
      const at = sectionAt(e.position);
      if (!at || e.paths.length === 0) return;
      const run: Promise<DropOutcome> = readOnly
        ? Promise.resolve({ kind: 'error', text: workingDropError('', 'read_only', '') })
        : at === 'working'
          ? dropOnWorking(e.paths)
          : dropInFolder(at, SECTION_LABEL[at], e.paths);
      void run.then((n) => {
        if (alive) setNotice({ ...n, at });
      });
    })
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => undefined);
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [readOnly]);
  return { dragging, over, notice };
}

function OpenProject({ project }: { project: ProjectSnapshot }) {
  const { revision, error } = useProjectState();
  const readOnly = project.read_only !== null;
  const [deleting, setDeleting] = useState(false);
  // The confirmed delete is on its way (a close question may be showing meanwhile).
  const [removing, setRemoving] = useState(false);
  // R97: a delete is refused while a job runs; say so on the button instead of after Confirm.
  const jobRunning = useActiveJobId() !== null;
  const drop = usePanelDrop(readOnly);
  const dropProps = (id: DropSection) => ({
    dropOver: drop.over === id,
    notice: drop.notice?.at === id ? drop.notice : null,
  });
  return (
    <div
      className={`prj-panel${drop.dragging ? ' prj-panel-dragging' : ''}`}
      data-testid="prj-panel"
      data-kind={project.kind}
      data-dragging={drop.dragging ? 'true' : undefined}
    >
      <header className="prj-panel-header">
        <div className="row">
          <button
            type="button"
            className="link"
            data-testid="prj-panel-close"
            title="Close the project and go back to the home grid"
            onClick={() => void projectStore.close()}
          >
            ← Home
          </button>
          <span className="pf-spacer" />
          <button
            type="button"
            className="link"
            data-testid="prj-panel-delete"
            disabled={jobRunning || removing}
            title={
              jobRunning
                ? 'A job is running in this project. Delete it when the job has finished.'
                : 'Delete the project. It stays in Recently deleted for 30 days.'
            }
            onClick={() => setDeleting(true)}
          >
            Delete
          </button>
        </div>
        <ProjectName project={project} />
      </header>
      <DeleteProjectDialog
        open={deleting}
        name={project.name}
        onOpenChange={setDeleting}
        confirm={async () => {
          // The confirmation came first (R111). It is modal, so it goes before the close
          // questions are asked (a running export asks in its own panel) and comes back only
          // to say why a delete was refused.
          setDeleting(false);
          setRemoving(true);
          const refused = await projectStore.remove();
          setRemoving(false);
          if (refused) setDeleting(true);
          return refused;
        }}
      />
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
      <Folder
        id="working"
        label="Working"
        revision={revision}
        canAdd={false}
        {...dropProps('working')}
      />
      {USER_FOLDERS.map((f) => (
        <Folder
          key={f.id}
          id={f.id}
          label={f.label}
          revision={revision}
          canAdd={!readOnly}
          {...dropProps(f.id as DropSection)}
        />
      ))}
      <Results revision={revision} readOnly={readOnly} />
      <History project={project} revision={revision} />
      <Storage revision={revision} readOnly={readOnly} />
    </div>
  );
}

/** The project's name, edited in place (FR-PRJ-56): click it, Enter commits, Esc cancels. The
 *  project stays open under its new name; a refusal (`name_taken`, `job_active`, …) shows under
 *  the field and nothing changes. */
function ProjectName({ project }: { project: ProjectSnapshot }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(project.name);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) return;
    input.current?.focus();
    input.current?.select();
  }, [editing]);
  // The field is disabled while the rename runs and loses focus: a refusal gives it back, so
  // Esc and typing work at once.
  useEffect(() => {
    if (error) input.current?.focus();
  }, [error]);

  if (!editing) {
    return (
      <h2 className="prj-panel-name">
        <button
          type="button"
          className="prj-panel-name-button"
          data-testid="prj-panel-name"
          title={`${project.project_dir}\nClick to rename`}
          onClick={() => {
            setValue(project.name);
            setError(null);
            setEditing(true);
          }}
        >
          {project.name}
        </button>
      </h2>
    );
  }

  const commit = async () => {
    if (busy) return;
    setBusy(true);
    const err = await projectStore.rename(value);
    setBusy(false);
    if (err) setError(err.message);
    else setEditing(false);
  };

  return (
    <form
      className="prj-panel-rename"
      onSubmit={(e) => {
        e.preventDefault();
        void commit();
      }}
    >
      <input
        ref={input}
        data-testid="prj-panel-name-input"
        aria-label="Project name"
        aria-invalid={error ? true : undefined}
        value={value}
        maxLength={64}
        disabled={busy}
        onChange={(e) => {
          setValue(e.target.value);
          setError(null);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            setEditing(false);
          }
        }}
        onBlur={() => {
          if (!error && !busy) setEditing(false);
        }}
      />
      {error && (
        <span className="error small" data-testid="prj-panel-name-error" role="alert">
          {error}
        </span>
      )}
    </form>
  );
}

function Section({
  id,
  label,
  count,
  actions,
  dropzone,
  children,
}: {
  id: string;
  label: string;
  count?: number;
  actions?: ReactNode;
  /** FR-PRJ-55: the section takes file drops; `over` = the drag is over it now. */
  dropzone?: { over: boolean };
  children: ReactNode;
}) {
  const details = (
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
  if (!dropzone) return details;
  return (
    <div
      className={`prj-dropzone${dropzone.over ? ' prj-dropzone-over' : ''}`}
      data-testid={`prj-panel-${id}-dropzone`}
      data-drop-section={id}
      data-over={dropzone.over ? 'true' : undefined}
    >
      {details}
    </div>
  );
}

// ------------------------------------------------------------------ warnings and relink

const WARNING_TEXT: Record<string, string> = {
  original_changed: 'Original changed: it was saved over by another editor. Read-only.',
  original_missing: 'Original missing. Relink it to keep editing.',
  base_changed: 'Base image changed: it was saved over or removed outside Pocket Forge. Read-only.',
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
            {/* A space before the link (walk 2 N4: "machine. Compare versions"). */}
            {(code === 'folder_moved' || code === 'conflict') && ' '}
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
  dropOver,
  notice,
}: {
  id: ProjectFolder;
  label: string;
  revision: number;
  canAdd: boolean;
  dropOver: boolean;
  /** What the last file drop on this section did (FR-PRJ-55). */
  notice: DropNotice | null;
}) {
  const [files, setFiles] = useState<FolderFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const seq = useProjectState().folders[id] ?? 0;
  const load = useCallback(() => {
    api.project.folderList(id).then(
      (f) => {
        setFiles(f);
        setError(null);
      },
      (e: unknown) => {
        // BL-30: no stale (or empty) list under the error, e.g. once the folder was moved.
        setFiles(null);
        setError(errorOf(e).message);
      },
    );
  }, [id]);
  useEffect(load, [load, revision, seq]);
  // Every list of this folder reloads (New image's sketches too, walk-gen L2).
  const changed = useCallback(() => projectStore.folderChanged(id), [id]);
  useOnFocus(changed);

  const add = async (paths: string[]) => {
    try {
      setFiles(await api.project.folderAdd(id, paths));
      setError(null);
      changed();
    } catch (e) {
      setError(errorOf(e).message);
    }
  };

  return (
    <Section
      id={id}
      label={label}
      count={files?.length}
      dropzone={{ over: dropOver }}
      actions={
        <>
          {canAdd && <AddFiles folder={id} onAdd={add} />}
          <button
            type="button"
            className="prj-icon-button"
            data-testid={`prj-panel-${id}-refresh`}
            title={`Refresh ${label}`}
            aria-label={`Refresh ${label}`}
            onClick={changed}
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
      {notice && (
        <p
          className={`${notice.kind === 'error' ? 'error' : 'muted'} small`}
          data-testid={`prj-panel-${id}-drop-notice`}
          data-kind={notice.kind}
          role="status"
        >
          {notice.text}
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

/** Iterations (`results/NNN <slug>/`), from RGN's list; RGN's tracker selects them. Each one
 *  opens to its kept candidates, with Discard (#56: no confirmation, the trash keeps it 30 days). */
function Results({ revision, readOnly }: { revision: number; readOnly: boolean }) {
  const [items, setItems] = useState<RgnIteration[]>([]);
  const [cands, setCands] = useState<CandidateRecord[]>([]);
  const [notice, setNotice] = useState<{ kind: 'info' | 'error'; text: string } | null>(null);
  // #63: the iteration whose Discard all waits for its confirm.
  const [asking, setAsking] = useState<{ it: RgnIteration; left: number } | null>(null);
  const trash = useProjectState().trashNotice;
  const load = useCallback(() => {
    // The active job before it has candidates has no number or folder yet: not a result.
    api.region.iterations().then(
      (rows) => setItems(rows.filter((r) => r.n !== null)),
      () => setItems([]),
    );
    api.project.get().then(
      (p) =>
        setCands(
          (p as (ProjectSnapshot & { candidates?: CandidateRecord[] }) | null)?.candidates ?? [],
        ),
      () => setCands([]),
    );
  }, []);
  useEffect(load, [load, revision]);
  const discard = (c: CandidateRecord) =>
    api.region.discard(c.job_id, c.id).then(
      () => {
        // Walk owner-UX L3: one notice, shown here (the tracker's and Storage's go).
        setNotice(null);
        projectStore.setTrashNotice({
          at: 'results',
          text: 'Discarded: in the trash for 30 days (Storage → Restore).',
        });
        load();
      },
      (e: unknown) => setNotice({ kind: 'error', text: sentence(errorOf(e).message) }),
    );
  // #61: every kept result of the iteration to the trash (never the accepted one), after a
  // confirm (#63); the notice offers Undo, which puts each back as it was (star included). walk-61 L2: "N
  // results restored." is the one trash notice too, so a later discard anywhere replaces it.
  const undoAll = (undoId: string) => {
    projectStore.setTrashNotice(null);
    return api.region.undoDiscardAll(undoId).then(
      ({ restored }) => {
        setNotice(null);
        projectStore.setTrashNotice({
          at: 'results',
          text: `${restored} ${restored === 1 ? 'result' : 'results'} restored.`,
        });
        load();
      },
      (e: unknown) => setNotice({ kind: 'error', text: sentence(errorOf(e).message) }),
    );
  };
  const discardAll = (it: RgnIteration) =>
    api.region.discardAll(it.job_id).then(
      ({ trashed, undo_id }) => {
        setNotice(null);
        projectStore.setTrashNotice({
          at: 'results',
          text:
            trashed === 0
              ? 'Nothing to discard.'
              : `${trashed} ${trashed === 1 ? 'result' : 'results'} in the trash for 30 days`,
          ...(undo_id ? { undo: () => void undoAll(undo_id) } : {}),
        });
        load();
      },
      (e: unknown) => setNotice({ kind: 'error', text: sentence(errorOf(e).message) }),
    );
  return (
    <Section id="results" label="Results" count={items.length}>
      {notice ? (
        <p
          className={`${notice.kind === 'error' ? 'error' : 'muted'} small`}
          data-testid="prj-results-notice"
          role="status"
        >
          {notice.text}
        </p>
      ) : (
        trash?.at === 'results' && (
          <p className="muted small" data-testid="prj-results-notice" role="status">
            {trash.text}
            {trash.undo && (
              <>
                {' '}
                <button
                  type="button"
                  className="link"
                  data-testid="prj-results-discard-undo"
                  onClick={trash.undo}
                >
                  Undo
                </button>
              </>
            )}
          </p>
        )
      )}
      {items.length === 0 ? (
        <p className="muted small">No iterations yet</p>
      ) : (
        <ul className="prj-files">
          {items.map((it) => {
            const kept = cands
              .filter(
                (c) => c.job_id === it.job_id && c.trash_id === null && c.status !== 'trashed',
              )
              .sort((a, b) => a.index - b.index);
            return (
              <li
                key={it.job_id}
                data-testid="prj-panel-results-item"
                data-record-id={it.job_id}
                title={it.dir ?? undefined}
              >
                <details className="prj-result">
                  <summary className="prj-file">
                    <span className="prj-file-name">
                      {String(it.n).padStart(3, '0')} {it.slug ?? ''}
                    </span>
                    <span className="prj-file-size tabular">
                      {it.kept} / {it.returned}
                      {it.accepted_in_stack ? ' ✓' : ''}
                    </span>
                  </summary>
                  <div className="prj-result-actions">
                    <button
                      type="button"
                      className="link"
                      data-testid="prj-results-discard-all"
                      data-record-id={it.job_id}
                      disabled={readOnly || running(it.state) || discardable(it, kept) === 0}
                      title={
                        running(it.state)
                          ? 'The job is still running'
                          : 'Discard all: every result but the accepted one to the trash for 30 days'
                      }
                      onClick={() => setAsking({ it, left: discardable(it, kept) })}
                    >
                      Discard all
                    </button>
                  </div>
                  <ul className="prj-files prj-result-cands">
                    {kept.map((c) => (
                      <li
                        key={c.id}
                        className="prj-file"
                        data-testid="prj-results-candidate"
                        data-record-id={c.id}
                        data-status={c.status}
                        title={c.file}
                      >
                        <span className="prj-file-name">
                          {c.file.split('/').pop()}
                          {c.status === 'accepted' ? ' ✓' : ''}
                          {c.starred ? ' ★' : ''}
                        </span>
                        <button
                          type="button"
                          className="link"
                          data-testid="prj-results-discard"
                          disabled={readOnly || c.status === 'accepted'}
                          title={
                            c.status === 'accepted'
                              ? 'The accepted candidate can’t be discarded'
                              : 'Discard: to the trash for 30 days'
                          }
                          onClick={() => void discard(c)}
                        >
                          Discard
                        </button>
                      </li>
                    ))}
                  </ul>
                </details>
              </li>
            );
          })}
        </ul>
      )}
      <ConfirmAsk
        open={asking !== null}
        testid="prj-results-discard-all-ask"
        title={
          asking
            ? `Discard ${asking.left === 1 ? 'the 1 result' : `all ${asking.left} results`} of ${String(asking.it.n).padStart(3, '0')}?`
            : ''
        }
        description={`${asking?.it.accepted_in_stack ? 'The accepted one stays. ' : ''}They go to the trash for 30 days.`}
        confirmLabel="Discard all"
        onCancel={() => setAsking(null)}
        onConfirm={() => {
          const it = asking?.it;
          setAsking(null);
          if (it) void discardAll(it);
        }}
      />
    </Section>
  );
}

/** A job still running (RGN's tracker rule): not terminal and not `fetched`. */
function running(state: RgnIteration['state']): boolean {
  return !['fetched', 'failed', 'canceled', 'lost', 'closed'].includes(state);
}

/** What Discard all would bin (#61): RGN's count, else the listed results but the accepted one. */
function discardable(it: RgnIteration, kept: CandidateRecord[]): number {
  return it.discardable ?? kept.filter((c) => c.status !== 'accepted').length;
}

// ------------------------------------------------------------------ history

function History({ project, revision }: { project: ProjectSnapshot; revision: number }) {
  const [revs, setRevs] = useState<RevisionEntry[]>(project.revisions);
  const [label, setLabel] = useState('');
  const { save } = useProjectState();
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
      {save.kind === 'unchanged' && (
        // Next to the button, not just in the status bar (BL-27); the next commit clears it.
        <p className="muted small" data-testid="prj-panel-history-unchanged" role="status">
          No changes since the last revision
        </p>
      )}
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
  const projectState = useProjectState();
  const moved = folderMoved(projectState);
  const notice = projectState.trashNotice?.at === 'storage' ? projectState.trashNotice : null;
  /** Walk owner-UX L1: any call failing with `folder_moved` clears the figures and the trash
   *  list (not only a revision change), and every feature hears of it. */
  const movedFailure = useCallback((e: unknown): boolean => {
    if (!isFolderMovedError(e)) return false;
    setUsage(null);
    setTrash([]);
    setError(FOLDER_MOVED_TEXT);
    projectStore.noteFolderMoved();
    return true;
  }, []);
  const load = useCallback(() => {
    api.project.diskUsage().then(
      (u) => {
        setUsage(u);
        setError(null);
      },
      (e: unknown) => {
        if (movedFailure(e)) return;
        const err = errorOf(e);
        // BL-30: no stale (or 0 B) figures under the error.
        setUsage(null);
        // `ipc`: the core doesn't register the command yet (PRJ slice 5): say so quietly.
        setError(err.code === 'ipc' ? null : err.message);
        setUnavailable(err.code === 'ipc');
      },
    );
    api.project.trashList().then(setTrash, (e: unknown) => {
      if (!movedFailure(e)) setTrash([]);
    });
  }, [movedFailure]);
  // The folder moved (or came back): reload, so the figures never outlive it.
  useEffect(load, [load, revision, moved]);

  const total = usage ? Object.values(usage.bytes).reduce((s, b) => s + b, 0) : 0;
  return (
    <Section id="storage" label="Storage">
      {error && <p className="error small">{error}</p>}
      {notice && (
        <p className="muted small" data-testid="prj-panel-storage-notice" role="status">
          {notice.text}
        </p>
      )}
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
                  <dt>{storageLabel(k, projectState.project)}</dt>
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
              title={trashFiles(t).join('\n')}
            >
              <span className="prj-file-name">{trashLabel(t)}</span>
              <span className="prj-file-size tabular">{bytesLabel(t.bytes)}</span>
              {t.restorable && (
                <button
                  type="button"
                  className="link"
                  data-testid="prj-panel-storage-restore"
                  disabled={readOnly}
                  onClick={() =>
                    api.project.trashRestore(t.id).then(
                      () => {
                        // Walk owner-UX L3: replaces the stale "Discarded…" notice.
                        projectStore.setTrashNotice({
                          at: 'storage',
                          text: `Restored ${trashLabel(t)}.`,
                        });
                        load();
                      },
                      (e: unknown) => {
                        if (!movedFailure(e)) setError(sentence(errorOf(e).message));
                      },
                    )
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

/** Every project path in a trash item (`files`, else the first `path`). */
function trashFiles(t: TrashItem): string[] {
  return t.files && t.files.length > 0 ? t.files : [t.path];
}

/** BL-30: the first file's name, plus how many more the item holds. Walk owner-UX L3: a
 *  result file names its iteration ("002 · 03.png"); a superseded mask says so, not its id. */
function trashLabel(t: TrashItem): string {
  const files = trashFiles(t);
  const first = files[0]!;
  const iteration = /^results\/(\d{3})[^/]*\/([^/]+)$/.exec(first);
  const name =
    t.kind === 'mask'
      ? t.reason === 'superseded'
        ? 'Mask (superseded)'
        : 'Mask'
      : iteration
        ? `${iteration[1]} · ${iteration[2]}`
        : first.split('/').pop();
  return files.length > 1 ? `${name} +${files.length - 1} more` : (name ?? '');
}

/** A core message as a sentence for the panel (walk 2 N2: no raw lower-case text). */
function sentence(message: string): string {
  const m = message.trim();
  return m ? `${m[0]!.toUpperCase()}${m.slice(1)}${/[.!?]$/.test(m) ? '' : '.'}` : m;
}
