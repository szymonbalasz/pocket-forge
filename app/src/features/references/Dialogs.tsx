// Pack dialogs (reference-packs §7): a name dialog (+ New pack, Rename) and the delete confirm.
// Radix Dialog: Esc closes, the first field (or the safe button) has the default focus.
import { useState } from 'react';
import { Dialog } from 'radix-ui';

/** + New pack / Rename (FR-REF-07, 10). `submit` returns the field error, or null when done. */
export function NameDialog({
  open,
  title,
  action,
  initial = '',
  onOpenChange,
  submit,
}: {
  open: boolean;
  title: string;
  action: string;
  initial?: string;
  onOpenChange(open: boolean): void;
  submit(name: string): Promise<string | null>;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="ref-overlay" />
        <Dialog.Content
          className="ref-dialog"
          data-testid="ref-dialog-name"
          aria-describedby={undefined}
        >
          <Dialog.Title asChild>
            <h2>{title}</h2>
          </Dialog.Title>
          {open && (
            <NameForm
              initial={initial}
              action={action}
              submit={submit}
              done={() => onOpenChange(false)}
            />
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function NameForm({
  initial,
  action,
  submit,
  done,
}: {
  initial: string;
  action: string;
  submit(name: string): Promise<string | null>;
  done(): void;
}) {
  const [name, setName] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        void submit(name.trim()).then((err) => {
          setBusy(false);
          if (err) setError(err);
          else done();
        });
      }}
    >
      <input
        aria-label="Pack name"
        data-testid="ref-dialog-name-input"
        value={name}
        autoFocus
        maxLength={64}
        onFocus={(e) => e.currentTarget.select()}
        onChange={(e) => {
          setName(e.target.value);
          setError(null);
        }}
      />
      {error && (
        <p className="error small" role="alert" data-testid="ref-dialog-name-error">
          {error}
        </p>
      )}
      <div className="row">
        <span className="pf-spacer" />
        <Dialog.Close asChild>
          <button type="button">Cancel</button>
        </Dialog.Close>
        <button
          type="submit"
          className="primary"
          data-testid="ref-dialog-name-submit"
          disabled={!name.trim() || busy}
        >
          {action}
        </button>
      </div>
    </form>
  );
}

/** Delete a pack → the References trash, restorable for 30 days (FR-REF-12). */
export function DeleteDialog({
  open,
  name,
  onOpenChange,
  confirm,
}: {
  open: boolean;
  name: string;
  onOpenChange(open: boolean): void;
  confirm(): Promise<unknown>;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="ref-overlay" />
        <Dialog.Content className="ref-dialog" data-testid="ref-dialog-delete">
          <Dialog.Title asChild>
            <h2>Delete “{name}”?</h2>
          </Dialog.Title>
          <Dialog.Description className="muted small">
            The pack moves to Recently deleted. You can restore it for 30 days.
          </Dialog.Description>
          <div className="row">
            <span className="pf-spacer" />
            <Dialog.Close asChild>
              <button type="button" autoFocus data-testid="ref-dialog-delete-cancel">
                Cancel
              </button>
            </Dialog.Close>
            <button
              type="button"
              className="primary"
              data-testid="ref-dialog-delete-confirm"
              onClick={() => void confirm().then(() => onOpenChange(false))}
            >
              Delete
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
