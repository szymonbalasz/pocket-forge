// A small modal "are you sure?" for one-click actions that are easy to hit by accident (#64:
// Retry, Discard all). Cancel has the focus, so Enter or Escape backs out.
import type { ReactNode, SyntheticEvent } from 'react';
import { Dialog } from 'radix-ui';

export function ConfirmAsk({
  open,
  title,
  description,
  confirmLabel,
  testid,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  description: ReactNode;
  confirmLabel: string;
  /** Test id of the dialog; its buttons are `<testid>-confirm` and `<testid>-cancel`. */
  testid: string;
  onConfirm(): void;
  onCancel(): void;
}) {
  // The dialog is portalled, but React events still bubble to the owner's handlers (a tracker
  // chip selects its iteration on click), so they stop here.
  const stop = (e: SyntheticEvent) => e.stopPropagation();
  return (
    <Dialog.Root open={open} onOpenChange={(next) => !next && onCancel()}>
      <Dialog.Portal>
        <Dialog.Overlay className="prj-dialog-overlay" onClick={stop} />
        <Dialog.Content className="prj-dialog" data-testid={testid} onClick={stop} onKeyDown={stop}>
          <Dialog.Title asChild>
            <h2>{title}</h2>
          </Dialog.Title>
          <Dialog.Description className="muted small">{description}</Dialog.Description>
          <div className="row">
            <span className="pf-spacer" />
            <button type="button" autoFocus data-testid={`${testid}-cancel`} onClick={onCancel}>
              Cancel
            </button>
            <button
              type="button"
              className="primary"
              data-testid={`${testid}-confirm`}
              onClick={onConfirm}
            >
              {confirmLabel}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
