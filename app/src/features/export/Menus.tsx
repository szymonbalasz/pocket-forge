// The image menus other features place (#63, EXP §7), built from the shared component, and the
// toast menu exports report in. The toast is mounted with the status bar, so it shows on Home
// too (a pack image's Save as).
import { useEffect, useState, type ReactElement } from 'react';
import { createPortal } from 'react-dom';

import {
  ContextMenu,
  imageMenuEntries,
  type MenuEntry,
  type MenuFormat,
  type MenuItem,
} from '../../shared/ContextMenu';
import { projectHasImage } from '../../shared/generateTabs';
import { useProjectState } from '../../store/project';
import { menuExport, useMenuToast } from './menu';
import { exportStore, useExportState } from './store';
import { ago, errorText, fileNameOf, formatBytes } from './view';

const NO_IMAGE = 'No image yet';

/** EDT's canvas menu (`edt-canvas-menu`, FR-EXP-30, 31, FR-PRJ-59): Save as…, Export as ▸ at the
 *  panel's size, and Open in Explorer for the project folder. Opening it asks for the decoded
 *  original. */
export function CanvasMenu({ children }: { children: ReactElement }) {
  const { project } = useProjectState();
  const { plan } = useExportState();
  if (!project) return children;
  const noImage = !projectHasImage(project);
  const formatDisabled: Partial<Record<MenuFormat, string | null>> = {};
  for (const f of plan?.formats ?? []) formatDisabled[f.format] = f.available ? null : f.reason;
  const entries = imageMenuEntries({
    onSaveAs: () => void menuExport.canvasSaveAs(),
    saveAsDisabled: noImage ? NO_IMAGE : null,
    onExport: (f) => void menuExport.canvasExportAs(f),
    exportDisabled: noImage ? NO_IMAGE : project.read_only ? errorText('read_only') : null,
    formatDisabled,
    onReveal: () => void menuExport.reveal(project.project_dir),
  });
  return (
    <ContextMenu
      testId="edt-canvas-menu"
      entries={entries}
      onOpenChange={(open) => {
        if (open && !noImage) exportStore.prefetchOriginal();
      }}
    >
      {children}
    </ContextMenu>
  );
}

/** A project file's or candidate's menu (FR-EXP-32, FR-PRJ-59): an image gets Save as…,
 *  Export as ▸ (into `exports/`; disabled on a read-only project) and Open in Explorer; any other
 *  file Open in Explorer only. `path` is absolute. */
export function FileMenu({
  testId,
  path,
  image,
  children,
}: {
  testId: string;
  path: string;
  image: boolean;
  children: ReactElement;
}) {
  const { project } = useProjectState();
  const onReveal = () => void menuExport.reveal(path);
  const entries = image
    ? imageMenuEntries({
        onSaveAs: () => void menuExport.fileSaveAs(path),
        onExport: (f) => void menuExport.fileExportAs(path, f),
        exportDisabled: project?.read_only ? errorText('read_only') : null,
        onReveal,
      })
    : imageMenuEntries({ onReveal });
  return (
    <ContextMenu testId={testId} entries={entries}>
      {children}
    </ContextMenu>
  );
}

/** A pack image's menu (`ref-pack-image-menu`, REF §7): Save as… and Open in Explorer. A pack
 *  has no `exports/`, so there is no Export as. */
export function PackImageMenu({ path, children }: { path: string; children: ReactElement }) {
  const entries = imageMenuEntries({
    onSaveAs: () => void menuExport.fileSaveAs(path),
    onReveal: () => void menuExport.reveal(path),
  });
  return (
    <ContextMenu testId="ref-pack-image-menu" entries={entries}>
      {children}
    </ContextMenu>
  );
}

/** "Open in Explorer" for a card or the panel name's menu (FR-PRJ-59). */
export function revealEntry(path: string): MenuItem {
  return { id: 'reveal', label: 'Open in Explorer', onSelect: () => void menuExport.reveal(path) };
}

export type { MenuEntry };

/** The menu exports' toast (`exp-toast`, `data-source="menu"`): progress and Cancel, Replace on
 *  `exists`, the path with Copy path when done, or the error. */
export function MenuToastView() {
  const t = useMenuToast();
  const [copied, setCopied] = useState<'idle' | 'copied' | 'failed'>('idle');
  const doneAt = t?.phase === 'done' ? t.seq : null;
  useEffect(() => {
    setCopied('idle');
    if (doneAt === null) return;
    const id = window.setTimeout(() => {
      if (menuExport.get()?.seq === doneAt) menuExport.dismiss();
    }, 10_000);
    return () => window.clearTimeout(id);
  }, [doneAt]);
  if (!t || typeof document === 'undefined') return null;

  const copy = async () => {
    if (!t.done) return;
    try {
      await navigator.clipboard.writeText(t.done.path);
      setCopied('copied');
    } catch {
      setCopied('failed');
    }
  };
  const close = (
    <button
      type="button"
      className="link"
      data-testid="exp-toast-close"
      onClick={() => menuExport.dismiss()}
    >
      Close
    </button>
  );
  let body: ReactElement;
  if (t.phase === 'exists' && t.exists) {
    body = (
      <>
        <span className="exp-toast-title" data-testid="exp-toast-question">
          Replace {t.exists.name} ({formatBytes(t.exists.bytes)}, {ago(t.exists.mtime, Date.now())}
          )?
        </span>
        <div className="row">
          <button
            type="button"
            className="primary"
            data-testid="exp-toast-replace"
            onClick={() => void menuExport.replace()}
          >
            Replace
          </button>
          <button
            type="button"
            data-testid="exp-toast-cancel"
            onClick={() => void menuExport.cancel()}
          >
            Cancel
          </button>
        </div>
      </>
    );
  } else if (t.phase === 'done' && t.done) {
    body = (
      <>
        <span className="exp-toast-title">
          {t.job?.kind === 'canvas-save' || (t.job?.kind === 'file' && t.job.dest !== 'exports')
            ? 'Saved'
            : 'Exported'}{' '}
          {fileNameOf(t.done.path)}
        </span>
        <span className="small muted tabular" data-testid="exp-toast-detail">
          {t.done.width}×{t.done.height} · {formatBytes(t.done.bytes)}
          <span className="exp-toast-path" data-testid="exp-toast-path">
            {t.done.path}
          </span>
        </span>
        {t.done.fallback === 'lossless' && (
          <p className="small warning" data-testid="exp-toast-fallback">
            Saved as lossless WebP: the lossy encoder couldn't handle this image.
          </p>
        )}
        <div className="row">
          <button type="button" data-testid="exp-toast-copy" onClick={() => void copy()}>
            {copied === 'copied' ? 'Copied' : copied === 'failed' ? "Couldn't copy" : 'Copy path'}
          </button>
          {close}
        </div>
      </>
    );
  } else if (t.phase === 'failed') {
    body = (
      <>
        <span className="exp-toast-title">{t.label}</span>
        <span className="small error" data-testid="exp-toast-error">
          {t.error}
        </span>
        <div className="row">{close}</div>
      </>
    );
  } else {
    body = (
      <>
        <span className="exp-toast-title" data-testid="exp-toast-label">
          {t.canceling ? 'Canceling…' : t.label}
        </span>
        {t.notice && (
          <span className="small warning" data-testid="exp-toast-notice" role="alert">
            {t.notice}
          </span>
        )}
        <div
          className="exp-progress"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(t.fraction * 100)}
          data-testid="exp-toast-progress"
        >
          <div className="exp-progress-fill" style={{ width: `${t.fraction * 100}%` }} />
        </div>
        <div className="row">
          <button
            type="button"
            data-testid="exp-toast-cancel"
            disabled={t.canceling || (t.phase === 'running' && !t.exportId)}
            onClick={() => void menuExport.cancel()}
          >
            Cancel
          </button>
        </div>
      </>
    );
  }
  return createPortal(
    <div
      className={`exp-toast exp-menu-toast${t.phase === 'failed' ? ' exp-toast-failed' : ''}`}
      data-testid="exp-toast"
      data-source="menu"
      data-phase={t.phase}
      role="status"
    >
      {body}
    </div>,
    document.body,
  );
}
