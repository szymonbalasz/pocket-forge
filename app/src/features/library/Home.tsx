// The home grid (asset-project §7, FR-PRJ-03, 49): shown in the canvas slot while no project is
// open. Project cards of the selected section, + New, a full-area drop zone and paste. Also the
// first-run root picker and the "Library unavailable" banner (FR-PRJ-01, 02).
import { useEffect, useState } from 'react';
import { Popover } from 'radix-ui';

import { onFileDrop, pickPaths } from '../../api';
import { useHomeView } from '../../shared/home';
import { IconPlus } from '../../shell/icons';
import { projectStore, useProjectState } from '../../store/project';
import { Card } from './Card';
import { SECTIONS } from './format';
import { isTextTarget, parsePastedPaths } from './paste';
import { libraryStore, useLibraryState } from './store';
import { fileName, reasonLabel, summarise } from './summary';
import './library.css';

export function Home() {
  const { project } = useProjectState();
  const view = useHomeView();
  // References shows REF's packs home in this slot instead (FR-PRJ-52).
  if (project || view === 'references') return null;
  return <HomeView />;
}

function HomeView() {
  const lib = useLibraryState();
  const { error, busy, opening } = useProjectState();
  const [over, setOver] = useState(false);
  const section = SECTIONS.find((s) => s.id === lib.section)!;

  useEffect(() => {
    void libraryStore.init();
  }, []);

  // Native file drops on the window (FR-PRJ-03, 07): image files and folders.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    onFileDrop((e) => {
      setOver(e.type === 'over');
      if (e.type === 'drop' && libraryStore.get().config?.configured)
        void libraryStore.create(e.paths);
    })
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => undefined);
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  // Paste (FR-PRJ-03, 10): image data, or a list of file paths. Text fields keep their paste.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if (isTextTarget(e.target) || !libraryStore.get().config?.configured) return;
      const data = e.clipboardData;
      if (!data) return;
      const image = [...data.items].find((i) => i.kind === 'file' && i.type.startsWith('image/'));
      const file = image?.getAsFile();
      if (file) {
        e.preventDefault();
        void file.arrayBuffer().then((b) => libraryStore.createFromImage(new Uint8Array(b)));
        return;
      }
      const paths = parsePastedPaths(data.getData('text/plain'));
      if (paths.length) {
        e.preventDefault();
        void libraryStore.create(paths);
      }
    };
    document.addEventListener('paste', onPaste);
    return () => document.removeEventListener('paste', onPaste);
  }, []);

  if (lib.config && lib.choosingRoot) return <RootSetup first={!lib.config.configured} />;

  const unavailable =
    lib.listError?.code === 'library_unavailable' ||
    (lib.config?.roots.some((r) => !r.available) ?? false);

  return (
    <div
      className={`prj-home${over ? ' prj-home-over' : ''}`}
      data-testid="prj-home-dropzone"
      data-over={over ? 'true' : undefined}
      data-section={lib.section}
      aria-busy={busy || undefined}
    >
      {unavailable && (
        <div className="banner banner-error" data-testid="prj-library-unavailable" role="alert">
          Library unavailable
          {lib.config?.roots.length ? `: ${lib.config.roots.map((r) => r.path).join(', ')}` : ''}
          <span className="pf-spacer" />
          <button
            type="button"
            className="link"
            data-testid="prj-library-choose-again"
            onClick={() => libraryStore.chooseRoot(true)}
          >
            Choose again
          </button>
          <button
            type="button"
            className="link"
            data-testid="prj-library-retry"
            onClick={() => void libraryStore.init()}
          >
            Retry
          </button>
        </div>
      )}
      {error && (
        <div className="banner banner-error" data-testid="prj-error" data-code={error.code}>
          {error.code === 'not_available_offline'
            ? 'Not available offline: this project is still in the cloud.'
            : error.message}
          <span className="pf-spacer" />
          {error.code === 'not_available_offline' && opening && (
            <button
              type="button"
              className="link"
              data-testid="prj-error-retry"
              onClick={() => void projectStore.open(opening)}
            >
              Retry
            </button>
          )}
          <button type="button" className="link" onClick={() => projectStore.clearError()}>
            Dismiss
          </button>
        </div>
      )}
      <header className="prj-home-header">
        <h1 data-testid="prj-home-title">{section.label}</h1>
        <span className="pf-spacer" />
        <NewButton disabled={unavailable || !!lib.creating} />
      </header>
      {lib.cards && lib.cards.length === 0 && !unavailable && (
        <div className="prj-home-empty" data-testid="prj-home-empty">
          <p>
            Drop an image to start {/^[aeiou]/.test(section.noun) ? 'an' : 'a'} {section.noun}
          </p>
          <p className="muted small">
            Drop image files or a folder here, paste an image, or use + New. Your files are copied;
            the originals stay where they are.
          </p>
        </div>
      )}
      {lib.cards && lib.cards.length > 0 && (
        <ul className="prj-grid" data-testid="prj-home-grid">
          {lib.cards.map((c) => (
            <Card key={c.id} card={c} />
          ))}
        </ul>
      )}
      {busy && opening && (
        <div className="prj-toast" data-testid="prj-home-opening" role="status">
          Opening {fileName(opening)}… (cloud files download first)
        </div>
      )}
      <CreationToast />
      {over && (
        <div className="prj-home-drop-hint" aria-hidden="true">
          Drop to create {section.label.toLowerCase()}
        </div>
      )}
    </div>
  );
}

/** + New (FR-PRJ-03): the native multi-select image dialog (#53). Where no dialog is available
 *  (tests on the mocked core), a popover takes file or folder paths, one per line. */
function NewButton({ disabled }: { disabled: boolean }) {
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
          className="primary"
          data-testid="prj-home-new"
          disabled={disabled}
          aria-haspopup="dialog"
          onClick={() =>
            pickPaths({ title: 'New from images', multiple: true, images: true }).then(
              (picked) => void libraryStore.create(picked),
              () => setOpen(true),
            )
          }
        >
          <IconPlus /> New
        </button>
      </Popover.Anchor>
      <Popover.Portal>
        <Popover.Content className="prj-popover" align="end" sideOffset={6}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              setOpen(false);
              setText('');
              void libraryStore.create(paths);
            }}
          >
            <label className="small muted" htmlFor="prj-new-paths">
              Image or folder paths, one per line
            </label>
            <textarea
              id="prj-new-paths"
              data-testid="prj-home-new-paths"
              rows={4}
              value={text}
              autoFocus
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && (e.ctrlKey || e.metaKey))
                  e.currentTarget.form?.requestSubmit();
              }}
            />
            <div className="row">
              <span className="pf-spacer" />
              <button
                type="submit"
                className="primary"
                data-testid="prj-home-new-create"
                disabled={paths.length === 0}
              >
                Create
              </button>
            </div>
          </form>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

/** Progress "12 / 40" with Cancel, then the summary (asset-project §7). No dialog. */
function CreationToast() {
  const { creating, summary } = useLibraryState();
  if (creating) {
    return (
      <div className="prj-toast" data-testid="prj-import-progress" role="status">
        <span className="tabular">
          Creating {creating.done} / {creating.total}
        </span>
        <button
          type="button"
          data-testid="prj-import-cancel"
          onClick={() => void libraryStore.cancelCreate()}
        >
          Cancel
        </button>
      </div>
    );
  }
  if (!summary) return null;
  const notes = summary.items.filter((i) => i.status !== 'created');
  return (
    <div className="prj-toast prj-toast-summary" data-testid="prj-import-summary" role="status">
      <div className="row">
        <span data-testid="prj-import-summary-text">
          {summarise(summary.items, summary.canceled)}
        </span>
        <span className="pf-spacer" />
        <button
          type="button"
          className="link"
          data-testid="prj-import-summary-dismiss"
          onClick={() => libraryStore.dismissSummary()}
        >
          Dismiss
        </button>
      </div>
      {notes.length > 0 && (
        <ul className="prj-summary-list">
          {notes.map((i) => (
            <li key={i.source} data-testid="prj-import-summary-item" data-status={i.status}>
              <span className="prj-summary-file" title={i.source}>
                {fileName(i.source)}
              </span>
              {i.status === 'skipped' ? (
                <span className="muted">{reasonLabel(i.reason)}</span>
              ) : (
                <>
                  <span className="muted">already in library</span>
                  <button
                    type="button"
                    className="link"
                    data-testid="prj-import-summary-open"
                    onClick={() => void projectStore.open(i.path)}
                  >
                    Open
                  </button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** First run: "Choose your library folder" (FR-PRJ-01); also "Choose again". */
function RootSetup({ first }: { first: boolean }) {
  const [path, setPath] = useState('');
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="prj-home prj-setup" data-testid="prj-setup">
      <form
        className="prj-setup-form"
        onSubmit={(e) => {
          e.preventDefault();
          setError(null);
          void libraryStore.setRoot(path).then(setError);
        }}
      >
        <h1>Choose your library folder</h1>
        <p className="muted">
          Pocket Forge keeps your maps, tokens and items here, one folder per project. A
          cloud-synced folder works.
        </p>
        <input
          data-testid="prj-setup-root-path"
          aria-label="Library folder"
          placeholder="Folder path"
          value={path}
          autoFocus
          onChange={(e) => {
            setPath(e.target.value);
            setError(null);
          }}
        />
        <button
          type="button"
          data-testid="prj-setup-browse"
          onClick={() =>
            pickPaths({ title: 'Choose your library folder', directory: true }).then(
              ([dir]) => {
                if (!dir) return;
                setPath(dir);
                setError(null);
                void libraryStore.setRoot(dir).then(setError);
              },
              () => setError('No folder dialog here: type the path instead.'),
            )
          }
        >
          Browse…
        </button>
        {error && (
          <p className="error small" data-testid="prj-setup-error" role="alert">
            {error}
          </p>
        )}
        <div className="row">
          {!first && (
            <button type="button" onClick={() => libraryStore.chooseRoot(false)}>
              Cancel
            </button>
          )}
          <span className="pf-spacer" />
          <button
            type="submit"
            className="primary"
            data-testid="prj-setup-pick-root"
            disabled={!path.trim()}
          >
            Use this folder
          </button>
        </div>
      </form>
    </div>
  );
}
