// REF's home slot view (FR-PRJ-52, reference-packs §7): the packs home (cards, + New pack, folder
// drop, Recently deleted) and the pack view (name, notes, Add…, Refresh, Delete, image grid with
// roles). Shown in the canvas slot while References is selected and no project is open.
import { useEffect, useState } from 'react';
import { ContextMenu, Popover } from 'radix-ui';

import { onFileDrop, pickPaths, type PackCard, type PackImage } from '../../api';
import { dateLabel } from '../../shared/dates';
import { useHomeView } from '../../shared/home';
import { IconPlus, IconReferences } from '../../shell/icons';
import { useProjectState } from '../../store/project';
import { DeleteDialog, NameDialog } from './Dialogs';
import {
  badgeLabel,
  folderName,
  imagesLabel,
  isTextTarget,
  parsePastedPaths,
  reasonLabel,
  ROLES,
  summariseAdd,
  trashLabel,
} from './format';
import { RoleSelect } from './RoleSelect';
import { refs, useRefs } from './store';
import './references.css';

export function ReferencesHome() {
  const { project } = useProjectState();
  const view = useHomeView();
  if (project || view !== 'references') return null;
  return <ReferencesView />;
}

function ReferencesView() {
  const s = useRefs();
  const [over, setOver] = useState(false);

  // Re-list on open and on window focus, so files added in Explorer appear (FR-REF-09).
  useEffect(() => {
    void refs.refresh();
    const onFocus = () => void refs.refresh();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, []);

  // Native drops: on the packs home a folder makes a pack (FR-REF-07); on a pack view the files
  // are copied in (FR-REF-08).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    onFileDrop((e) => {
      setOver(e.type === 'over');
      if (e.type !== 'drop') return;
      if (refs.get().openId) void refs.addImages(e.paths);
      else void refs.createFromFolders(e.paths);
    })
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => undefined);
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  // A pasted file list adds to the open pack (FR-REF-08). Text fields keep their paste.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if (isTextTarget(e.target) || !refs.get().openId || !e.clipboardData) return;
      const paths = parsePastedPaths(e.clipboardData.getData('text/plain'));
      if (paths.length) {
        e.preventDefault();
        void refs.addImages(paths);
      }
    };
    document.addEventListener('paste', onPaste);
    return () => document.removeEventListener('paste', onPaste);
  }, []);

  return (
    <div
      className="ref-home"
      data-testid={s.openId ? 'ref-pack-dropzone' : 'ref-home-dropzone'}
      data-over={over ? 'true' : undefined}
    >
      {s.error && (
        <div className="banner banner-error" data-testid="ref-error" data-code={s.error.code}>
          {s.error.message}
          <span className="pf-spacer" />
          <button type="button" className="link" onClick={() => refs.clearError()}>
            Dismiss
          </button>
        </div>
      )}
      {s.openId ? <PackView /> : <PacksHome />}
      <AddToast />
      {over && (
        <div className="ref-meta" aria-hidden="true" style={{ textAlign: 'center' }}>
          {s.openId ? 'Drop to copy the images into this pack' : 'Drop a folder to make a pack'}
        </div>
      )}
    </div>
  );
}

// ── Packs home ───────────────────────────────────────────────────────────────────────────────

function PacksHome() {
  const s = useRefs();
  const [creating, setCreating] = useState(false);
  return (
    <>
      <header className="ref-header">
        <h1 data-testid="ref-home-title">References</h1>
        <span className="pf-spacer" />
        <button
          type="button"
          className="primary"
          data-testid="ref-home-new"
          onClick={() => setCreating(true)}
        >
          <IconPlus /> New pack
        </button>
      </header>
      {s.listError && (
        <div className="banner banner-error" data-testid="ref-home-list-error">
          {s.listError.message}
        </div>
      )}
      <ListNotices />
      {s.cards && s.cards.length === 0 && (
        <div className="ref-empty" data-testid="ref-home-empty">
          <p>Create a pack, or drop a folder of reference images</p>
          <p className="muted small">
            A pack is a folder in your library’s References folder. Images are copied in; the
            originals stay where they are.
          </p>
        </div>
      )}
      {s.cards && s.cards.length > 0 && (
        <ul className="ref-grid" data-testid="ref-home-grid">
          {s.cards.map((c) => (
            <PackCardView key={c.id} card={c} />
          ))}
        </ul>
      )}
      <TrashList />
      <NameDialog
        open={creating}
        title="New pack"
        action="Create"
        onOpenChange={setCreating}
        submit={(name) => refs.create(name)}
      />
    </>
  );
}

function PackCardView({ card }: { card: PackCard }) {
  // The URL that failed, not a flag: a new thumbnail (a new URL) is tried again.
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const thumbFailed = !!card.thumb_url && failedUrl === card.thumb_url;
  const [renaming, setRenaming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  // No `pack.json` yet (FR-REF-04): listed, but it can't be opened, renamed or deleted until a
  // later listing adopts it.
  const id = card.pack_id;
  return (
    <>
      <ContextMenu.Root>
        <ContextMenu.Trigger asChild disabled={!id}>
          <li
            className="ref-card"
            data-testid="ref-home-card"
            data-record-id={card.id}
            title={
              // BL-35: relative to the library root, never the absolute path.
              id
                ? (card.rel_path ?? card.name)
                : `${card.rel_path ?? card.name} (not ready: Pocket Forge couldn’t write pack.json)`
            }
          >
            <button
              type="button"
              className="ref-card-body"
              disabled={!id}
              onClick={() => id && void refs.open(id)}
              onKeyDown={(e) => {
                if (e.key === 'F2' && id) {
                  e.preventDefault();
                  setRenaming(true);
                }
              }}
            >
              {card.thumb_url && !thumbFailed ? (
                <img
                  className="ref-thumb"
                  src={card.thumb_url}
                  alt=""
                  loading="lazy"
                  data-testid="ref-home-card-thumb"
                  onError={() => setFailedUrl(card.thumb_url)}
                />
              ) : (
                <span className="ref-thumb ref-thumb-empty" data-testid="ref-home-card-thumb">
                  <IconReferences size={28} />
                </span>
              )}
              <span className="ref-card-name" data-testid="ref-home-card-name">
                {card.name}
              </span>
              <span className="ref-card-line tabular" data-testid="ref-home-card-count">
                {imagesLabel(card.image_count)}
              </span>
              {card.notes_first_line && (
                <span className="ref-card-line" data-testid="ref-home-card-notes">
                  {card.notes_first_line}
                </span>
              )}
            </button>
            {card.badges.length > 0 && (
              <span className="ref-badges">
                {card.badges.map((b) => (
                  <span
                    key={b}
                    className={`ref-badge ref-badge-${b}`}
                    data-testid="ref-home-card-badge"
                    data-badge={b}
                  >
                    {badgeLabel(b)}
                  </span>
                ))}
              </span>
            )}
          </li>
        </ContextMenu.Trigger>
        <ContextMenu.Portal>
          <ContextMenu.Content className="ref-menu" data-testid="ref-home-card-menu">
            <ContextMenu.Item
              className="ref-menu-item"
              data-testid="ref-home-card-rename"
              onSelect={() => setRenaming(true)}
            >
              Rename
            </ContextMenu.Item>
            <ContextMenu.Item
              className="ref-menu-item"
              data-testid="ref-home-card-delete"
              onSelect={() => setDeleting(true)}
            >
              Delete
            </ContextMenu.Item>
          </ContextMenu.Content>
        </ContextMenu.Portal>
      </ContextMenu.Root>
      {id && (
        <>
          <NameDialog
            open={renaming}
            title="Rename pack"
            action="Rename"
            initial={card.name}
            onOpenChange={setRenaming}
            submit={(name) => refs.rename(id, name)}
          />
          <DeleteDialog
            open={deleting}
            name={card.name}
            onOpenChange={setDeleting}
            confirm={() => refs.deletePack(id)}
          />
        </>
      )}
    </>
  );
}

/** What the last listing did (adopted a folder, gave a copied pack a new id, resolved a
 *  `pack.json` conflict, FR-REF-04, 05, 28), from the cards' `notices[]`. */
function ListNotices() {
  const { notices: shown } = useRefs();
  if (!shown.length) return null;
  return (
    <div className="banner banner-warning" role="status" data-testid="ref-home-notices">
      <ul className="ref-trash">
        {shown.map((n) => (
          <li key={n.id} data-testid="ref-home-notice" data-code={n.code}>
            {n.message}
          </li>
        ))}
      </ul>
      <span className="pf-spacer" />
      <button
        type="button"
        className="link"
        onClick={() => refs.dismissNotices(shown.map((n) => n.id))}
      >
        Dismiss
      </button>
    </div>
  );
}

/** Recently deleted (FR-REF-12): packs and images in the References trash, with Restore. */
function TrashList() {
  const { trash } = useRefs();
  if (!trash?.length) return null;
  return (
    <section>
      <h2>Recently deleted</h2>
      <ul className="ref-trash" data-testid="ref-trash-list">
        {trash.map((t) => (
          <li key={t.id} data-testid="ref-trash-item" data-record-id={t.id} data-kind={t.kind}>
            <span>{trashLabel(t)}</span>
            <span className="ref-meta tabular" title={`Deleted ${dateLabel(t.trashed_at)}`}>
              until {dateLabel(t.purge_at)}
            </span>
            {t.restorable !== false && (
              <button
                type="button"
                className="link"
                data-testid="ref-trash-restore"
                onClick={() => void refs.restore(t.id)}
              >
                Restore
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

// ── Pack view ────────────────────────────────────────────────────────────────────────────────

function PackView() {
  const { pack, packError, openId } = useRefs();
  const [renaming, setRenaming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  if (packError)
    return (
      <>
        <BackButton />
        <div className="banner banner-error" data-testid="ref-pack-error">
          {packError.code === 'unknown_id' ? 'This pack is gone.' : packError.message}
        </div>
      </>
    );
  if (!pack || pack.pack_id !== openId) return <BackButton />;
  return (
    <>
      <header className="ref-header">
        <BackButton />
        <button
          type="button"
          className="ref-title-button"
          title="Rename"
          data-testid="ref-pack-name"
          onClick={() => setRenaming(true)}
        >
          {pack.name}
        </button>
        <span className="ref-meta tabular" data-testid="ref-pack-count">
          {imagesLabel(pack.images.length)}
        </span>
        <span className="pf-spacer" />
        <AddButton />
        <button type="button" data-testid="ref-pack-refresh" onClick={() => void refs.refresh()}>
          Refresh
        </button>
        <button type="button" data-testid="ref-pack-delete" onClick={() => setDeleting(true)}>
          Delete
        </button>
      </header>
      <Notes key={pack.pack_id} notes={pack.notes} />
      {pack.images.length === 0 ? (
        <div className="ref-empty" data-testid="ref-pack-empty">
          <p>No images yet</p>
          <p className="muted small">Drop images here, paste their paths, or use Add….</p>
        </div>
      ) : (
        <ul className="ref-grid ref-grid-images" data-testid="ref-pack-grid">
          {pack.images.map((img) => (
            <ImageCard key={img.file} img={img} />
          ))}
        </ul>
      )}
      <NameDialog
        open={renaming}
        title="Rename pack"
        action="Rename"
        initial={pack.name}
        onOpenChange={setRenaming}
        submit={(name) => refs.rename(pack.pack_id, name)}
      />
      <DeleteDialog
        open={deleting}
        name={pack.name}
        onOpenChange={setDeleting}
        confirm={() => refs.deletePack(pack.pack_id)}
      />
    </>
  );
}

function BackButton() {
  return (
    <button type="button" data-testid="ref-pack-back" onClick={() => refs.close()}>
      ← Packs
    </button>
  );
}

/** Notes (≤ 2000 chars, never sent, #9): saved on blur in one `pack.json` write (FR-REF-11). */
function Notes({ notes }: { notes: string }) {
  const [text, setText] = useState(notes);
  // The notes as last shown: a change from outside (Explorer, another machine, then Refresh)
  // replaces the text unless the DM has unsaved typing, so a blur never writes stale text back.
  const [shown, setShown] = useState(notes);
  if (notes !== shown) {
    setShown(notes);
    if (text === shown) setText(notes);
  }
  return (
    <textarea
      className="ref-notes"
      aria-label="Notes"
      placeholder="Notes (for you only; never sent)"
      data-testid="ref-pack-notes"
      maxLength={2000}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        if (text !== notes) void refs.setNotes(text);
      }}
    />
  );
}

const STATE_BADGE: Record<string, string | null> = {
  ok: null,
  cloud_only: 'cloud',
  corrupt: 'corrupt',
  unsupported: 'unsupported',
};

function ImageCard({ img }: { img: PackImage }) {
  // The URL that failed, not a flag: a refreshed file (a new URL) is tried again.
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const failed = !!img.thumb_url && failedUrl === img.thumb_url;
  const badge = STATE_BADGE[img.state] ?? null;
  return (
    <li
      className="ref-card"
      data-testid="ref-pack-image"
      data-record-id={img.file}
      data-state={img.state}
    >
      {img.thumb_url && !failed ? (
        // Lazy: only visible thumbnails load (FR-REF-14).
        <img
          className="ref-thumb"
          src={img.thumb_url}
          alt=""
          loading="lazy"
          onError={() => setFailedUrl(img.thumb_url ?? null)}
        />
      ) : (
        <span className="ref-thumb ref-thumb-empty">
          <IconReferences size={24} />
        </span>
      )}
      {badge && (
        <span className="ref-badges">
          <span className={`ref-badge ref-badge-${img.state}`} data-testid="ref-pack-image-badge">
            {badge}
          </span>
        </span>
      )}
      <span className="ref-card-name" title={img.file}>
        {img.file}
      </span>
      <div className="ref-image-foot">
        <RoleSelect
          value={img.role ?? null}
          roles={ROLES}
          allowNone
          label={`Role of ${img.file}`}
          testId="ref-pack-image-role"
          onChange={(role) => void refs.setRole(img.file, role)}
        />
        <button
          type="button"
          className="link small"
          data-testid="ref-pack-image-remove"
          onClick={() => void refs.removeImages([img.file])}
        >
          Remove
        </button>
      </div>
    </li>
  );
}

/** Add… (FR-REF-08): the native multi-select image dialog (#53). Where no dialog is available
 *  (tests on the mocked core), a popover takes paths, one per line. */
function AddButton() {
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
          data-testid="ref-pack-add"
          aria-haspopup="dialog"
          onClick={() =>
            pickPaths({ title: 'Add images to the pack', multiple: true, images: true }).then(
              (picked) => void refs.addImages(picked),
              () => setOpen(true),
            )
          }
        >
          Add…
        </button>
      </Popover.Anchor>
      <Popover.Portal>
        <Popover.Content className="ref-popover" align="end" sideOffset={6}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              setOpen(false);
              setText('');
              void refs.addImages(paths);
            }}
          >
            <label className="small muted" htmlFor="ref-add-paths">
              Image paths, one per line
            </label>
            <textarea
              id="ref-add-paths"
              data-testid="ref-pack-add-paths"
              rows={4}
              value={text}
              autoFocus
              onChange={(e) => setText(e.target.value)}
            />
            <div className="row">
              <span className="pf-spacer" />
              <button
                type="submit"
                className="primary"
                data-testid="ref-pack-add-confirm"
                disabled={paths.length === 0}
              >
                Add
              </button>
            </div>
          </form>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

/** Copy progress, then the per-item summary (FR-REF-08). No dialog. */
function AddToast() {
  const { adding, summary } = useRefs();
  if (adding)
    return (
      <div className="ref-toast" data-testid="ref-add-progress" role="status">
        <span className="tabular">
          Copying {adding.done} / {adding.total}
        </span>
      </div>
    );
  if (!summary) return null;
  const notes = summary.items.filter((i) => i.status !== 'added');
  return (
    <div className="ref-toast" data-testid="ref-add-summary" role="status">
      <div className="row">
        <span data-testid="ref-add-summary-text">{summariseAdd(summary.items)}</span>
        <span className="pf-spacer" />
        <button type="button" className="link" onClick={() => refs.dismissSummary()}>
          Dismiss
        </button>
      </div>
      {notes.length > 0 && (
        <ul>
          {notes.map((i) => (
            <li key={i.source} data-testid="ref-add-summary-item" data-status={i.status}>
              <span title={i.source}>{folderName(i.source)}</span>
              <span className="muted">
                {i.status === 'skipped' ? reasonLabel(i.reason) : `already in pack as ${i.file}`}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
