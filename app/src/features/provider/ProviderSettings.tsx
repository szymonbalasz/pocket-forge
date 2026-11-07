// Provider settings (provider spec §7): key and secret as password fields (cleared after Save),
// Save, Clear, Test connection, optional team and project id. The secret is never read back.
// A Radix dialog (ui-guidelines): Esc closes, the key field has the default focus.
import { useEffect, useState, type FormEvent } from 'react';
import { Dialog } from 'radix-ui';

import { api, ApiError } from '../../api';
import { providerStore, useProviderState } from './store';

type Result = { state: 'ok' | 'error'; text: string } | null;

function messageOf(e: unknown): string {
  if (!(e instanceof ApiError)) return String(e);
  const provider = (e.detail as { provider?: { code?: string } } | undefined)?.provider?.code;
  switch (provider ?? e.code) {
    case 'auth':
      return 'Scenario rejected the key and secret.';
    case 'offline':
      return 'Scenario can’t be reached (offline).';
    case 'timeout':
      return 'Scenario didn’t answer in time.';
    case 'not_configured':
      return 'Enter a key and secret first.';
    case 'store_unavailable':
      return 'The OS credential store is unavailable.';
    default:
      return e.message;
  }
}

function SettingsForm() {
  const { status } = useProviderState();
  const [key, setKey] = useState('');
  const [secret, setSecret] = useState('');
  const [team, setTeam] = useState('');
  const [project, setProject] = useState('');
  const [busy, setBusy] = useState<'save' | 'clear' | 'test' | null>(null);
  const [result, setResult] = useState<Result>(null);

  useEffect(() => {
    api.provider.getSettings().then(
      (s) => {
        setTeam(s.team_id ?? '');
        setProject(s.project_id ?? '');
      },
      () => undefined,
    );
  }, []);

  const run = async (kind: 'save' | 'clear' | 'test', fn: () => Promise<string>) => {
    setBusy(kind);
    setResult(null);
    try {
      setResult({ state: 'ok', text: await fn() });
    } catch (e) {
      setResult({ state: 'error', text: messageOf(e) });
    } finally {
      setBusy(null);
      void providerStore.refresh();
    }
  };

  const save = (e: FormEvent) => {
    e.preventDefault();
    void run('save', async () => {
      if (key || secret) {
        try {
          await api.provider.setCredentials(key, secret);
        } finally {
          // Never keep the secret in the UI longer than needed.
          setKey('');
          setSecret('');
        }
      }
      await api.provider.setSettings(team.trim() || null, project.trim() || null);
      return 'Saved.';
    });
  };

  return (
    <form onSubmit={save}>
      <p className="prv-meta" data-testid="prv-settings-stored">
        {status?.store_unavailable
          ? 'The credential store is unavailable.'
          : status?.key_suffix
            ? `A key ending in …${status.key_suffix} is stored.`
            : 'No key stored.'}
      </p>
      <label className="prv-field">
        API key
        <input
          type="password"
          autoComplete="off"
          data-testid="prv-settings-key"
          value={key}
          onChange={(e) => setKey(e.target.value)}
        />
      </label>
      <label className="prv-field">
        API secret
        <input
          type="password"
          autoComplete="off"
          data-testid="prv-settings-secret"
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
        />
      </label>
      <label className="prv-field">
        Team id (optional, for the balance)
        <input
          data-testid="prv-settings-team"
          value={team}
          onChange={(e) => setTeam(e.target.value)}
        />
      </label>
      <label className="prv-field">
        Project id (optional)
        <input
          data-testid="prv-settings-project"
          value={project}
          onChange={(e) => setProject(e.target.value)}
        />
      </label>
      <div className="prv-actions">
        <button
          type="submit"
          className="primary"
          data-testid="prv-settings-save"
          disabled={busy !== null}
        >
          Save
        </button>
        <button
          type="button"
          data-testid="prv-settings-clear"
          disabled={busy !== null}
          onClick={() =>
            void run('clear', async () => {
              await api.provider.clearCredentials();
              return 'Key and secret removed.';
            })
          }
        >
          Clear
        </button>
        <button
          type="button"
          data-testid="prv-settings-test"
          data-busy={busy === 'test'}
          disabled={busy !== null}
          onClick={() =>
            void run('test', async () => {
              await api.provider.testConnection();
              return 'Connected.';
            })
          }
        >
          {busy === 'test' ? (
            <span className="prv-spinner" data-testid="prv-settings-spinner" aria-label="Testing" />
          ) : null}
          Test connection
        </button>
      </div>
      {result && (
        <p
          className={result.state === 'ok' ? 'prv-meta' : 'prv-meta prv-error'}
          data-testid="prv-settings-result"
          data-state={result.state}
        >
          {result.text}
        </p>
      )}
    </form>
  );
}

export function ProviderSettings() {
  const { settingsOpen } = useProviderState();
  return (
    <Dialog.Root
      open={settingsOpen}
      onOpenChange={(open) => (open ? providerStore.openSettings() : providerStore.closeSettings())}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="prv-overlay" />
        <Dialog.Content
          className="prv-settings"
          data-testid="prv-settings-panel"
          aria-describedby={undefined}
        >
          <header className="prv-settings-head">
            <Dialog.Title className="prv-title">Scenario provider</Dialog.Title>
          </header>
          <SettingsForm />
          <Dialog.Close asChild>
            <button type="button" className="link prv-close" data-testid="prv-settings-close">
              Close
            </button>
          </Dialog.Close>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
