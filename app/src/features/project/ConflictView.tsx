// The conflict view (asset-project §7, FR-PRJ-36, #39): both versions side by side with their time,
// machine and layer stack, and Keep this / Keep other. A side whose files haven't synced shows
// "Still syncing" and can't be kept. EDT can't draw a given layer stack read-only yet
// (OQ-PRJ-11 A), so each side lists its layers as text (the B fallback).
import { useEffect, useState } from 'react';
import { Dialog } from 'radix-ui';

import { api, type ConflictSide, type ConflictView as View, type ConflictKeep } from '../../api';
import { errorOf, projectStore } from '../../store/project';
import { revisionLabel, timeLabel } from './format';

export function ConflictView({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Dialog.Root open={open} onOpenChange={(o) => !o && onClose()}>
      {open && <Body onClose={onClose} />}
    </Dialog.Root>
  );
}

function Body({ onClose }: { onClose: () => void }) {
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.project.conflictGet().then(setView, (e: unknown) => setError(errorOf(e).message));
  }, []);

  const keep = (side: ConflictKeep) => {
    setBusy(true);
    setError(null);
    api.project.resolveConflict(side).then(
      async () => {
        await projectStore.resolved(side);
        onClose();
      },
      (e: unknown) => {
        const err = errorOf(e);
        setBusy(false);
        setError(
          err.code === 'files_missing'
            ? 'That version’s files are still syncing. Try again in a moment.'
            : err.message,
        );
        // A side may have finished syncing (or not): show the current state.
        api.project.conflictGet().then(setView, () => undefined);
      },
    );
  };

  return (
    <Dialog.Portal>
      <Dialog.Overlay className="prj-overlay" />
      <Dialog.Content className="prj-conflict" data-testid="prj-conflict">
        <Dialog.Title className="prj-conflict-title">
          This project was also changed on another machine
        </Dialog.Title>
        <Dialog.Description className="muted small">
          Keep one version. The other stays in the history as a revision you can restore.
        </Dialog.Description>
        {error && (
          <p className="error small" role="alert" data-testid="prj-conflict-error">
            {error}
          </p>
        )}
        {view ? (
          <div className="prj-conflict-panes">
            <Pane side={view.this} title="This version" busy={busy} onKeep={keep} />
            <Pane side={view.other} title="Other version" busy={busy} onKeep={keep} />
          </div>
        ) : (
          !error && <p className="muted small">Loading both versions…</p>
        )}
        <div className="prj-conflict-actions">
          <Dialog.Close asChild>
            <button type="button" data-testid="prj-conflict-later">
              Decide later
            </button>
          </Dialog.Close>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  );
}

function Pane({
  side,
  title,
  busy,
  onKeep,
}: {
  side: ConflictSide;
  title: string;
  busy: boolean;
  onKeep: (side: ConflictKeep) => void;
}) {
  const syncing = !side.readable || side.missing_files.length > 0;
  const rev = side.newest_revision;
  return (
    <section
      className="prj-conflict-pane"
      data-testid={`prj-conflict-${side.side}`}
      data-can-keep={side.can_keep ? 'true' : 'false'}
    >
      <h3>{title}</h3>
      <p className="small" data-testid={`prj-conflict-${side.side}-time`}>
        {side.committed_at ? `Saved ${timeLabel(side.committed_at)}` : 'Not readable yet'}
        {' · '}
        {side.machine === 'this' ? 'this machine' : 'another machine'}
      </p>
      {rev && (
        <p className="muted small">
          Newest revision r{rev.seq} · {revisionLabel(rev)}
        </p>
      )}
      <ol className="prj-conflict-layers" data-testid={`prj-conflict-${side.side}-layers`}>
        {side.layers.map((l, i) => (
          <li key={l.id} data-record-id={l.id}>
            {l.kind === 'original' ? 'Original' : `Patch ${i}`}
            {l.visible ? '' : ' (hidden)'}
          </li>
        ))}
      </ol>
      {syncing && (
        <p className="muted small" data-testid={`prj-conflict-${side.side}-syncing`}>
          Still syncing
          {side.missing_files.length > 0 ? ` (${side.missing_files.length} files missing)` : ''}
        </p>
      )}
      <button
        type="button"
        className="primary"
        data-testid={`prj-conflict-keep-${side.side}`}
        disabled={!side.can_keep || busy}
        onClick={() => onKeep(side.side)}
      >
        {side.side === 'this' ? 'Keep this' : 'Keep other'}
      </button>
    </section>
  );
}
