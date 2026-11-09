// Settings → Prompt chat (prompt-helper §7, FR-PRM-20/45, PRM slice 6): on/off, the backend
// status, Log in… / Log out with the command shown, the Claude Code path override and the
// privacy notice. Consent is only ever given in the notice (Accept).
import { useEffect, useState } from 'react';

import { api, onEvent, type ChatStatus } from '../../api';
import { backendChip, loginNote, PRIVACY_NOTICE } from './chatLogic';

export function ChatSettings() {
  const [status, setStatus] = useState<ChatStatus | null>(null);
  const [path, setPath] = useState('');
  const [notice, setNotice] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    void api.chat.status().then(
      (s) => {
        setStatus(s);
        setPath(s.claude_path ?? '');
      },
      () => undefined,
    );
    // L2: the status follows changes made elsewhere (the chat window, a Recheck, detection).
    const sub = onEvent('prm:chat-status', (p) => setStatus((s) => (s ? { ...s, ...p } : s)));
    return () =>
      void sub.then(
        (off) =>
          void Promise.resolve()
            .then(off)
            .catch(() => undefined),
        () => undefined,
      );
  }, []);

  const run = (p: Promise<ChatStatus | null>, ok?: string) =>
    void p.then(
      (s) => {
        if (s) setStatus(s);
        setMessage(ok ? { ok: true, text: ok } : null);
      },
      (e: { message?: string }) => setMessage({ ok: false, text: e.message ?? String(e) }),
    );

  if (!status) return <p className="prv-meta">Loading…</p>;
  const loggedIn = status.backend.state === 'ok';
  const fake = status.mode === 'fake';

  return (
    <div className="rgn-settings prm-chat-settings">
      <label className="prm-chat-check">
        <input
          type="checkbox"
          data-testid="prm-settings-chat-toggle"
          checked={status.enabled}
          onChange={(e) => run(api.chat.settingsPut({ enabled: e.target.checked }))}
        />
        Prompt chat
      </label>
      <p className="prv-meta" data-testid="prm-settings-chat-status">
        {!status.consent
          ? 'Not opted in: the privacy notice is shown before the first chat.'
          : !status.enabled
            ? 'Off.'
            : backendChip(status)}
        {fake && ' (fake backend)'}
      </p>
      {status.consent && status.backend.state === 'error' && (
        <p className="prv-meta">{status.backend.fix}</p>
      )}
      {status.consent && (
        <div className="prv-actions">
          <button
            type="button"
            className="primary"
            data-testid="prm-settings-chat-login"
            onClick={() =>
              run(
                api.chat.login().then(() => null),
                loginNote(status),
              )
            }
          >
            Log in…
          </button>
          <button
            type="button"
            data-testid="prm-settings-chat-logout"
            disabled={!loggedIn}
            onClick={() => run(api.chat.logout(), 'Logged out.')}
          >
            Log out
          </button>
          <button
            type="button"
            data-testid="prm-settings-chat-recheck"
            onClick={() => run(api.chat.status(true))}
          >
            Recheck
          </button>
        </div>
      )}
      {status.login_command && (
        <p className="prv-meta">
          Or run: <code>{status.login_command}</code>
        </p>
      )}
      <label className="prv-meta" htmlFor="prm-chat-path">
        Claude Code executable (optional)
      </label>
      <input
        id="prm-chat-path"
        data-testid="prm-settings-chat-path"
        placeholder="Found on PATH"
        value={path}
        onChange={(e) => setPath(e.target.value)}
        onBlur={() => run(api.chat.settingsPut({ claude_path: path.trim() || null }))}
      />
      <div className="prv-actions">
        <button type="button" onClick={() => setNotice((n) => !n)}>
          {notice ? 'Hide privacy notice' : 'Show privacy notice'}
        </button>
      </div>
      {notice && (
        <p className="prv-meta">
          <strong>{PRIVACY_NOTICE[0]}</strong> {PRIVACY_NOTICE[1]}
        </p>
      )}
      {message && <p className={message.ok ? 'prv-meta' : 'prv-meta prv-error'}>{message.text}</p>}
    </div>
  );
}
