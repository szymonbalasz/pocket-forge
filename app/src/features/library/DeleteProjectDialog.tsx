// The delete confirmation (FR-PRJ-57), asked from the card's context menu and from the left
// panel: "Delete <name>? It stays in Recently deleted for 30 days."
import { useState } from 'react';
import { Dialog } from 'radix-ui';

import './library.css';

export function DeleteProjectDialog({
  open,
  name,
  onOpenChange,
  confirm,
}: {
  open: boolean;
  name: string;
  onOpenChange(open: boolean): void;
  /** Deletes; resolves with the error to show (the dialog stays), or null (it closes). */
  confirm(): Promise<{ code: string; message: string } | null>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const change = (next: boolean) => {
    if (busy) return;
    setError(null);
    onOpenChange(next);
  };
  const run = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    const err = await confirm();
    setBusy(false);
    if (err) setError(err.message);
    else onOpenChange(false);
  };

  return (
    <Dialog.Root open={open} onOpenChange={change}>
      <Dialog.Portal>
        <Dialog.Overlay className="prj-dialog-overlay" />
        <Dialog.Content className="prj-dialog" data-testid="prj-delete-dialog">
          <Dialog.Title asChild>
            <h2 data-testid="prj-delete-title">Delete {name}?</h2>
          </Dialog.Title>
          <Dialog.Description className="muted small">
            It stays in Recently deleted for 30 days.
          </Dialog.Description>
          {error && (
            <p className="error small" data-testid="prj-delete-error" role="alert">
              {error}
            </p>
          )}
          <div className="row">
            <span className="pf-spacer" />
            <button
              type="button"
              autoFocus
              data-testid="prj-delete-cancel"
              disabled={busy}
              onClick={() => change(false)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="primary"
              data-testid="prj-delete-confirm"
              disabled={busy}
              onClick={() => void run()}
            >
              Delete
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
