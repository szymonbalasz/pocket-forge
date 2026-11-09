// + New → Generate new… (FR-GEN-01): asks for the project's name, prefilled `New map` /
// `New token` / `New item` (with ` (2)`, ` (3)`, … when taken, FR-PRJ-44), then
// `gen_project_create` and the project opens on the New image tab. PRJ's home renders it from
// its + New menu (through this feature's index.ts) and opens the created project.
import { useEffect, useState, type FormEvent } from 'react';
import { Dialog } from 'radix-ui';

import { api, ApiError, type Section } from '../../api';
import { generateTab, NEW_IMAGE_TAB } from '../../shared/generateTabs';
import './gen.css';

const DEFAULT_NAME: Record<Section, string> = {
  maps: 'New map',
  tokens: 'New token',
  items: 'New item',
};

/** The default name, made free among `taken` (case-insensitive, FR-PRJ-44). */
export function freeName(section: Section, taken: readonly string[]): string {
  const base = DEFAULT_NAME[section];
  const used = new Set(taken.map((n) => n.toLowerCase()));
  if (!used.has(base.toLowerCase())) return base;
  for (let i = 2; ; i++) {
    const n = `${base} (${i})`;
    if (!used.has(n.toLowerCase())) return n;
  }
}

const ERROR_TEXT: Record<string, string> = {
  invalid_name: 'Enter a name.',
  name_taken: 'A project with this name already exists here.',
  library_unavailable: 'The library folder is unavailable.',
};

export interface NewProjectDialogProps {
  open: boolean;
  section: Section;
  /** Project names already in the section (the prefill avoids them). */
  taken: readonly string[];
  onOpenChange(open: boolean): void;
  /** The project was created: open it (PRJ). */
  onCreated(path: string): void;
}

export function NewProjectDialog(props: NewProjectDialogProps) {
  return (
    <Dialog.Root open={props.open} onOpenChange={props.onOpenChange}>
      {props.open && <Body {...props} />}
    </Dialog.Root>
  );
}

function Body({ section, taken, onOpenChange, onCreated }: NewProjectDialogProps) {
  const [name, setName] = useState(() => freeName(section, taken));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setError(null), [name]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    const wanted = name.trim();
    if (!wanted) return setError(ERROR_TEXT.invalid_name!);
    setBusy(true);
    try {
      const { path } = await api.gen.projectCreate(section, wanted);
      // FR-GEN-01: the project opens with the New image tab active.
      generateTab.openNextWith(NEW_IMAGE_TAB);
      onOpenChange(false);
      onCreated(path);
    } catch (err) {
      const code = err instanceof ApiError ? err.code : 'io';
      setError(ERROR_TEXT[code] ?? (err instanceof Error ? err.message : String(err)));
      setBusy(false);
    }
  };

  return (
    <Dialog.Portal>
      <Dialog.Overlay className="rgn-overlay" />
      <Dialog.Content className="rgn-dialog gen-new-dialog" data-testid="gen-new">
        <Dialog.Title className="rgn-dialog-title">Generate new</Dialog.Title>
        <Dialog.Description className="muted small">
          Creates an empty project. Generate its first image in the New image tab.
        </Dialog.Description>
        <form onSubmit={(e) => void submit(e)}>
          <label className="gen-field">
            <span className="muted small">Name</span>
            <input
              data-testid="gen-new-name"
              aria-label="Name"
              value={name}
              autoFocus
              onFocus={(e) => e.currentTarget.select()}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          {error && (
            <p className="error small" data-testid="gen-new-error" role="alert">
              {error}
            </p>
          )}
          <div className="rgn-dialog-buttons">
            <Dialog.Close asChild>
              <button type="button" data-testid="gen-new-cancel">
                Cancel
              </button>
            </Dialog.Close>
            <button
              type="submit"
              className="primary"
              data-testid="gen-new-ok"
              disabled={busy || !name.trim()}
            >
              {busy ? 'Creating…' : 'Create'}
            </button>
          </div>
        </form>
      </Dialog.Content>
    </Dialog.Portal>
  );
}
