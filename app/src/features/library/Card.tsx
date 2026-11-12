// A home-grid project card (FR-PRJ-49): thumbnail, name, iteration count, last edited, badges.
// Click opens; the context menu (the shared one, #63) opens it, shows it in Explorer
// (FR-PRJ-59), renames the closed project in place (or F2, FR-PRJ-45) and deletes it after a
// confirmation (FR-PRJ-57).
import { useEffect, useRef, useState } from 'react';

import type { ProjectCard } from '../../api';
import { ContextMenu } from '../../shared/ContextMenu';
import { projectStore } from '../../store/project';
import { revealEntry } from '../export';
import { DeleteProjectDialog } from './DeleteProjectDialog';
import { BADGE_LABEL, editedLabel, iterationsLabel } from './format';
import { libraryStore } from './store';

export function Card({ card }: { card: ProjectCard }) {
  const [renaming, setRenaming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  // The URL that failed, not a flag: a new thumbnail (a new URL) is tried again (FR-PRJ-50).
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const thumbFailed = !!card.thumb_url && failedUrl === card.thumb_url;
  const onThumbFail = () => setFailedUrl(card.thumb_url);

  return (
    <>
      <ContextMenu
        testId="prj-home-card-menu"
        itemPrefix="prj-home-card"
        entries={[
          { id: 'open', label: 'Open', onSelect: () => void projectStore.open(card.path) },
          revealEntry(card.path),
          { id: 'rename', label: 'Rename', hint: 'F2', onSelect: () => setRenaming(true) },
          { id: 'delete', label: 'Delete', onSelect: () => setDeleting(true) },
        ]}
      >
        <li
          className="prj-card"
          data-testid="prj-home-card"
          data-record-id={card.id}
          data-path={card.path}
        >
          {renaming ? (
            <div className="prj-card-body">
              <Thumb card={card} failed={thumbFailed} onFail={onThumbFail} />
              <RenameField card={card} onDone={() => setRenaming(false)} />
            </div>
          ) : (
            <button
              type="button"
              className="prj-card-body"
              title={card.path}
              onClick={() => void projectStore.open(card.path)}
              onKeyDown={(e) => {
                if (e.key === 'F2') {
                  e.preventDefault();
                  setRenaming(true);
                }
              }}
            >
              <Thumb card={card} failed={thumbFailed} onFail={onThumbFail} />
              <span className="prj-card-name" data-testid="prj-home-card-name">
                {card.name}
              </span>
              <span className="prj-card-meta tabular">
                <span data-testid="prj-home-card-iterations">
                  {iterationsLabel(card.iterations)}
                </span>
                <span data-testid="prj-home-card-edited" title={card.last_edited ?? undefined}>
                  {editedLabel(card.last_edited)}
                </span>
              </span>
            </button>
          )}
          {card.badges.length > 0 && (
            <span className="prj-card-badges">
              {card.badges.map((b) => (
                <span
                  key={b}
                  className={`prj-badge prj-badge-${b}`}
                  data-testid="prj-home-card-badge"
                  data-badge={b}
                >
                  {BADGE_LABEL[b] ?? b}
                </span>
              ))}
            </span>
          )}
        </li>
      </ContextMenu>
      <DeleteProjectDialog
        open={deleting}
        name={card.name}
        onOpenChange={setDeleting}
        confirm={() => libraryStore.remove(card)}
      />
    </>
  );
}

function Thumb({
  card,
  failed,
  onFail,
}: {
  card: ProjectCard;
  failed: boolean;
  onFail: () => void;
}) {
  if (!card.thumb_url || failed) {
    // FR-GEN-03: a project with no image yet gets the placeholder (and its badge).
    const noImage = card.badges.includes('no_image');
    return (
      <span
        className="prj-card-thumb prj-card-thumb-empty"
        data-testid="prj-home-card-thumb"
        data-placeholder={noImage ? 'no_image' : undefined}
      >
        {card.name.slice(0, 1).toUpperCase()}
      </span>
    );
  }
  return (
    <img
      className="prj-card-thumb"
      data-testid="prj-home-card-thumb"
      src={card.thumb_url}
      alt=""
      loading="lazy"
      draggable={false}
      onError={onFail}
    />
  );
}

/** Inline rename: Enter commits, Esc cancels; errors (`name_taken`, …) stay under the field. */
function RenameField({ card, onDone }: { card: ProjectCard; onDone: () => void }) {
  const [value, setValue] = useState(card.name);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // Radix returns focus to the trigger when the menu closes; take it after that.
    const id = requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.select();
    });
    return () => cancelAnimationFrame(id);
  }, []);

  const commit = async () => {
    if (busy) return;
    setBusy(true);
    const err = await libraryStore.rename(card, value);
    setBusy(false);
    if (err) setError(err.message);
    else onDone();
  };

  return (
    <form
      className="prj-card-rename"
      onSubmit={(e) => {
        e.preventDefault();
        void commit();
      }}
    >
      <input
        ref={input}
        data-testid="prj-home-card-rename-input"
        aria-label="Project name"
        value={value}
        maxLength={64}
        onChange={(e) => {
          setValue(e.target.value);
          setError(null);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            onDone();
          }
        }}
        onBlur={() => {
          if (!error && !busy) onDone();
        }}
      />
      {error && (
        <span className="error small" data-testid="prj-home-card-rename-error" role="alert">
          {error}
        </span>
      )}
    </form>
  );
}
