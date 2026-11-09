// The prompt chat window (prompt-helper §7, FR-PRM-20..29, PRM slice 6): its own webview
// (label `prompt-chat`, R91), bound to the asset project open in the main window. Header
// (project · host · backend chip), transcript with proposal cards (Replace + Copy, #60), the
// context strip, the composer (Send / Stop) and the usage footer. Every state of §7: consent,
// off, backend errors with their fix and Recheck, no project, no prompt block, running,
// errors with Retry, offline, read-only, fake. It follows the main window through
// `prm:chat-context` (project and host changes, BL-41) and re-reads on focus.
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  api,
  emitEvent,
  onEvent,
  type ChatPreview,
  type ChatProposal,
  type ChatStatus,
  type ChatThreadView,
  type ChatTurn,
  type ProjectSnapshot,
} from '../../api';
import {
  backendChip,
  billsApiKey,
  HOST_LABEL,
  loginNote,
  PRIVACY_NOTICE,
  questionOf,
  segments,
  usageLabel,
} from './chatLogic';
import './chat.css';

type Problem = { code: string; message: string };

/** BL-42: how long a send waits for the main window to publish its pending context. */
export const FLUSH_WAIT_MS = 400;

/** BL-42 / FR-PRM-23: asks the main window to publish its pending context and waits for its
 *  answer (or {@link FLUSH_WAIT_MS}), so a send never misses the last edits. */
async function flushMainWindow(): Promise<void> {
  const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  let answered: () => void = () => undefined;
  const done = new Promise<void>((resolve) => {
    answered = resolve;
    setTimeout(resolve, FLUSH_WAIT_MS);
  });
  const off = await onEvent('prm:context-flushed', (p) => {
    if (p.id === id) answered();
  }).catch(() => null);
  try {
    await emitEvent('prm:context-flush', { id });
    await done;
  } catch {
    // No event bridge: send what the core has.
  } finally {
    if (off)
      void Promise.resolve()
        .then(off)
        .catch(() => undefined);
  }
}

function problemOf(e: unknown): Problem {
  const x = e as { code?: string; message?: string };
  return { code: x.code ?? 'error', message: x.message ?? String(e) };
}

export function ChatWindow() {
  const [status, setStatus] = useState<ChatStatus | null>(null);
  const [project, setProject] = useState<ProjectSnapshot | null | undefined>(undefined);
  const [view, setView] = useState<ChatThreadView | null>(null);
  const [live, setLive] = useState<{ turnId: string; text: string } | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [offline, setOffline] = useState(false);
  const [includeImages, setIncludeImages] = useState(true);
  const [declined, setDeclined] = useState(false);
  /** L2: what Log in… did ("A console opened…"). */
  const [loginSaid, setLoginSaid] = useState<string | null>(null);
  const [preview, setPreview] = useState<ChatPreview | null>(null);
  const [focusTick, setFocusTick] = useState(0);
  const [list, setList] = useState<HTMLOListElement | null>(null);
  /** M3: the transcript follows the newest turn unless the user scrolled up. */
  const stick = useRef(true);
  const deltas = useRef(new Map<string, string>());
  const ended = useRef(new Set<string>());
  const pid = project?.project_id ?? null;
  const pidRef = useRef(pid);
  pidRef.current = pid;

  const loadStatus = useCallback(async (refresh: boolean) => {
    try {
      const s = await api.chat.status(refresh);
      setStatus(s);
      if (refresh && s.backend.state === 'ok') setOffline(false);
    } catch (e) {
      setProblem(problemOf(e));
    }
  }, []);

  const loadThread = useCallback(async (id: string | null) => {
    if (!id) return setView(null);
    try {
      const v = await api.chat.thread(id);
      if (pidRef.current === id)
        setView((old) => (JSON.stringify(old) === JSON.stringify(v) ? old : v));
    } catch {
      // The project closed meanwhile: the next poll catches up.
    }
  }, []);

  const loadProject = useCallback(async () => {
    try {
      const p = await api.project.get();
      setProject((old) =>
        old?.project_id === p?.project_id && old?.read_only === p?.read_only ? old : p,
      );
    } catch {
      setProject(null);
    }
  }, []);

  // FR-PRM-33: detection on open and on window focus; the project and host follow the main
  // window.
  useEffect(() => {
    void loadStatus(false);
    void loadProject();
    const focus = () => {
      void loadStatus(true);
      void loadProject();
      setFocusTick((n) => n + 1);
    };
    window.addEventListener('focus', focus);
    return () => window.removeEventListener('focus', focus);
  }, [loadStatus, loadProject]);

  useEffect(() => {
    setLive(null);
    setProblem(null);
    void loadThread(pid);
  }, [pid, loadThread]);

  // FR-PRM-26 / BL-42: what the next turn would send, refreshed when the host, the thread or
  // Include images changes, and on focus.
  const host = view?.current_host ?? null;
  const turnCount = view?.thread?.turns.length ?? 0;
  useEffect(() => {
    if (!pid || !host) return setPreview(null);
    let stale = false;
    api.chat.preview(pid, includeImages).then(
      (p) => !stale && setPreview(p),
      () => !stale && setPreview(null),
    );
    return () => {
      stale = true;
    };
  }, [pid, host, includeImages, turnCount, focusTick]);

  // M3: the newest turn is in view on open, after a send and after a restart; a streaming reply
  // is followed while the transcript is at the bottom.
  useEffect(() => {
    stick.current = true;
  }, [pid, turnCount]);
  useEffect(() => {
    if (list && stick.current) list.scrollTop = list.scrollHeight;
  }, [list, pid, turnCount, view, live?.text]);

  useEffect(() => {
    const subs = [
      onEvent('prm:chat-delta', (d) => {
        deltas.current.set(d.turn_id, (deltas.current.get(d.turn_id) ?? '') + d.text);
        setLive((l) => (l && l.turnId === d.turn_id ? { ...l, text: l.text + d.text } : l));
      }),
      onEvent('prm:chat-turn', (t) => {
        ended.current.add(t.turn_id);
        setLive((l) => (l && l.turnId === t.turn_id ? null : l));
        if (t.error?.code === 'offline') setOffline(true);
        void loadThread(pidRef.current);
      }),
      // L2 / L6: detection, settings and consent changes are followed without a refocus.
      onEvent('prm:chat-status', (p) => setStatus((s) => (s ? { ...s, ...p } : s))),
      // BL-41: the main window opened, closed or switched project, or its visible tab changed.
      onEvent('prm:chat-context', (c) => {
        if (c.project_id !== pidRef.current) void loadProject();
        else void loadThread(c.project_id);
      }),
    ];
    return () =>
      subs.forEach(
        (s) =>
          void s.then(
            // The event bridge may be gone already (window closing): ignore.
            (off) =>
              void Promise.resolve()
                .then(off)
                .catch(() => undefined),
            () => undefined,
          ),
      );
  }, [loadThread, loadProject]);

  const send = async (text: string) => {
    if (!pid) return;
    setProblem(null);
    try {
      await flushMainWindow();
      const out = await api.chat.send(pid, text, includeImages);
      if (!ended.current.has(out.turn_id))
        setLive({ turnId: out.turn_id, text: deltas.current.get(out.turn_id) ?? '' });
      await loadThread(pid);
    } catch (e) {
      const p = problemOf(e);
      if (p.code === 'offline') setOffline(true);
      setProblem(p);
    }
  };

  const replace = async (p: ChatProposal) => {
    if (!pid) return;
    setProblem(null);
    try {
      await api.chat.insert(pid, p.id);
      await loadThread(pid);
    } catch (e) {
      setProblem(problemOf(e));
    }
  };

  if (!status)
    return (
      <main className="prm-chat" data-testid="prm-chat">
        <p className="muted">Loading…</p>
      </main>
    );

  const fake = status.mode === 'fake';
  const header = (
    <header className="prm-chat-head">
      <span className="prm-chat-title">{project?.name ?? 'Prompt chat'}</span>
      {view?.current_host && <span className="prm-chat-tag">{HOST_LABEL[view.current_host]}</span>}
      <span
        className={`prm-chat-chip${status.backend.state === 'error' ? ' is-error' : ''}`}
        data-testid="prm-chat-status"
      >
        {backendChip(status)}
      </span>
      {fake && (
        <span className="prm-chat-chip is-fake" data-testid="prm-chat-fake">
          Fake chat
        </span>
      )}
    </header>
  );

  if (!status.consent)
    return (
      <main className="prm-chat" data-testid="prm-chat">
        {header}
        <section className="prm-chat-notice" aria-label="Privacy notice">
          <p>
            <strong>{PRIVACY_NOTICE[0]}</strong> {PRIVACY_NOTICE[1]}
          </p>
          {declined && (
            <p className="muted small">The chat stays off. Accept the notice whenever you like.</p>
          )}
          <div className="prm-chat-actions">
            <button
              type="button"
              className="primary"
              data-testid="prm-chat-consent-accept"
              autoFocus
              onClick={() =>
                void api.chat.consent(true).then(setStatus, (e) => setProblem(problemOf(e)))
              }
            >
              Accept
            </button>
            <button
              type="button"
              data-testid="prm-chat-consent-decline"
              onClick={() => setDeclined(true)}
            >
              Not now
            </button>
          </div>
        </section>
      </main>
    );

  if (!status.enabled)
    return (
      <main className="prm-chat" data-testid="prm-chat">
        {header}
        <section className="prm-chat-notice">
          <p>Prompt chat is off.</p>
          <button
            type="button"
            className="primary"
            onClick={() => void api.chat.settingsPut({ enabled: true }).then(setStatus)}
          >
            Turn it on
          </button>
        </section>
      </main>
    );

  const b = status.backend;
  if (b.state === 'error')
    return (
      <main className="prm-chat" data-testid="prm-chat">
        {header}
        <section className="prm-chat-notice" role="alert">
          <p>{b.message}</p>
          <p className="muted">{b.fix}</p>
          {b.detail && <pre className="prm-chat-detail">{b.detail}</pre>}
          <div className="prm-chat-actions">
            {b.code === 'not_logged_in' && (
              <button
                type="button"
                className="primary"
                data-testid="prm-chat-login"
                onClick={() =>
                  void api.chat.login().then(
                    () => setLoginSaid(loginNote(status)),
                    (e: unknown) => setProblem(problemOf(e)),
                  )
                }
              >
                Log in…
              </button>
            )}
            <button
              type="button"
              data-testid="prm-chat-recheck"
              onClick={() => void loadStatus(true)}
            >
              Recheck
            </button>
          </div>
          {loginSaid && (
            <p className="muted small" role="status" data-testid="prm-chat-login-note">
              {loginSaid}
            </p>
          )}
          {problem && <p className="prm-chat-error">{problem.message}</p>}
        </section>
      </main>
    );

  if (!project)
    return (
      <main className="prm-chat" data-testid="prm-chat">
        {header}
        <p className="prm-chat-empty" data-testid="prm-chat-no-project">
          Open a project to chat about it.
        </p>
      </main>
    );

  const turns = view?.thread?.turns ?? [];
  const unsaved = new Set(view?.unsaved_turn_ids ?? []);
  const running = live?.turnId ?? view?.running_turn_id ?? null;
  const readOnly = !!project.read_only;
  const usage = usageLabel(turns);
  const blocked = !host
    ? 'Open the Generate panel to chat about this project.'
    : offline
      ? 'Offline: Recheck once the connection is back.'
      : null;

  return (
    <main className="prm-chat" data-testid="prm-chat">
      {header}
      <ol
        className="prm-chat-transcript"
        aria-label="Chat"
        data-testid="prm-chat-transcript"
        ref={setList}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
        }}
      >
        {turns.length === 0 && (
          <li className="prm-chat-empty">Ask for ideas about the prompt of the visible tab.</li>
        )}
        {turns.map((t) => (
          <TurnView
            key={t.id}
            turn={t}
            liveText={t.id === live?.turnId ? live.text : null}
            running={t.id === running}
            unsaved={unsaved.has(t.id)}
            readOnly={readOnly}
            onReplace={replace}
            retryBlocked={offline}
            onRetry={() => {
              const q = questionOf(turns, t.id);
              // M5: Retry respects the offline lock, as Send does.
              if (q && !offline) void send(q.text);
            }}
          />
        ))}
      </ol>
      {problem && (
        <p className="prm-chat-error" role="alert" data-testid="prm-chat-error">
          {problem.message}
        </p>
      )}
      <Strip
        host={host}
        includeImages={includeImages}
        onIncludeImages={setIncludeImages}
        freshNext={!!view?.fresh_session_next && turns.length > 0}
        apiKey={billsApiKey(status)}
        readOnly={readOnly}
        preview={preview}
      />
      <Composer
        disabledReason={blocked}
        onRecheck={offline ? () => void loadStatus(true) : undefined}
        running={running}
        onSend={send}
        onStop={() => {
          if (running) void api.chat.cancel(running).catch(() => undefined);
        }}
      />
      <footer className="prm-chat-foot muted small" data-testid="prm-chat-usage">
        {usage ?? 'No usage yet'}
      </footer>
    </main>
  );
}

function Strip(props: {
  host: ChatTurn['host'] | null;
  includeImages: boolean;
  onIncludeImages(on: boolean): void;
  freshNext: boolean;
  preview: ChatPreview | null;
  apiKey: boolean;
  readOnly: boolean;
}) {
  return (
    <div className="prm-chat-strip" data-testid="prm-chat-strip">
      <span className="prm-chat-tag" data-testid="prm-chat-host">
        {props.host ? HOST_LABEL[props.host] : 'No prompt block'}
      </span>
      <label className="prm-chat-check">
        <input
          type="checkbox"
          data-testid="prm-chat-include-images"
          checked={props.includeImages}
          onChange={(e) => props.onIncludeImages(e.target.checked)}
        />
        Include images
      </label>
      {props.preview && props.host && (
        <span className="prm-chat-preview" data-testid="prm-chat-preview">
          {props.preview.images.map((i) => (
            <img
              key={i.id}
              src={i.thumb}
              alt={i.already_sent ? `${i.label} (already sent)` : i.label}
              className={i.already_sent ? 'is-sent' : undefined}
            />
          ))}
          <span className="small muted">{props.preview.summary}</span>
          {props.preview.dropped.length > 0 && (
            <span className="prm-chat-warn small">
              Not sent (turn limit): {props.preview.dropped.join(', ')}
            </span>
          )}
        </span>
      )}
      {props.freshNext && (
        <span className="muted small">New session: earlier messages aren&apos;t sent.</span>
      )}
      {props.apiKey && (
        <span className="prm-chat-warn small">Billed to your Anthropic API key.</span>
      )}
      {props.readOnly && <span className="prm-chat-warn small">Not saved: read-only project.</span>}
    </div>
  );
}

function Composer(props: {
  disabledReason: string | null;
  /** M5: offline after a turn: Recheck lifts the lock once detection succeeds. */
  onRecheck?: () => void;
  running: string | null;
  onSend(text: string): Promise<void>;
  onStop(): void;
}) {
  const [text, setText] = useState('');
  const canSend = !props.disabledReason && !props.running && text.trim().length > 0;
  const send = () => {
    if (!canSend) return;
    const t = text;
    setText('');
    void props.onSend(t);
  };
  return (
    <div className="prm-chat-composer">
      <textarea
        data-testid="prm-chat-input"
        aria-label="Message"
        rows={3}
        maxLength={8000}
        value={text}
        placeholder="Ask about the prompt…"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && e.ctrlKey) {
            e.preventDefault();
            send();
          }
        }}
      />
      <div className="prm-chat-actions">
        {props.disabledReason && <span className="muted small">{props.disabledReason}</span>}
        {props.onRecheck && (
          <button type="button" data-testid="prm-chat-recheck" onClick={props.onRecheck}>
            Recheck
          </button>
        )}
        {props.running ? (
          <button type="button" data-testid="prm-chat-stop" onClick={props.onStop}>
            Stop
          </button>
        ) : (
          <button
            type="button"
            className="primary"
            data-testid="prm-chat-send"
            disabled={!canSend}
            aria-keyshortcuts="Control+Enter"
            onClick={send}
          >
            Send
          </button>
        )}
      </div>
    </div>
  );
}

function TurnView(props: {
  turn: ChatTurn;
  liveText: string | null;
  running: boolean;
  /** Kept in memory only (read-only project, §7 / R94). */
  unsaved: boolean;
  /** M5: offline: Retry and Resend wait for Recheck, as Send does. */
  retryBlocked: boolean;
  readOnly: boolean;
  onReplace(p: ChatProposal): Promise<void>;
  onRetry(): void;
}) {
  const t = props.turn;
  const notSaved = props.unsaved && (
    <span className="prm-chat-tag is-unsaved" data-testid="prm-chat-unsaved">
      Not saved
    </span>
  );
  if (t.role === 'user')
    return (
      <li className="prm-chat-turn is-user" data-record-id={t.id}>
        {t.fresh_session && <p className="prm-chat-session muted small">New session</p>}
        <span className="prm-chat-tag">{HOST_LABEL[t.host]}</span>
        {notSaved}
        <p className="prm-chat-text">{t.text}</p>
        {t.context && <span className="prm-chat-summary small muted">{t.context.summary}</span>}
        {(t.context?.dropped.length ?? 0) > 0 && (
          <span className="prm-chat-warn small" data-testid="prm-chat-dropped">
            Not sent (turn limit): {t.context!.dropped.join(', ')}
          </span>
        )}
      </li>
    );
  const text = props.liveText ?? t.text;
  return (
    <li className="prm-chat-turn is-reply" data-record-id={t.id} aria-busy={props.running}>
      {notSaved}
      {segments(text).map((s, i) =>
        s.kind === 'code' ? (
          <pre key={i} className="prm-chat-code" data-lang={s.lang || undefined}>
            <code>{s.text}</code>
          </pre>
        ) : (
          <p key={i} className="prm-chat-text">
            {s.text}
          </p>
        ),
      )}
      {props.running && <span className="prm-chat-caret" aria-label="Replying" />}
      {(t.proposals ?? []).map((p) => (
        <ProposalCard
          key={p.id}
          proposal={p}
          readOnly={props.readOnly}
          onReplace={props.onReplace}
        />
      ))}
      {t.state === 'canceled' && <p className="muted small">Stopped.</p>}
      {t.state === 'interrupted' && (
        <p className="muted small">
          Interrupted.{' '}
          <button
            type="button"
            className="link"
            disabled={props.retryBlocked}
            onClick={props.onRetry}
          >
            Resend
          </button>
        </p>
      )}
      {t.state === 'failed' && t.error && (
        <div className="prm-chat-error" role="alert">
          <p>{t.error.message}</p>
          {t.error.fix && <p className="muted small">{t.error.fix}</p>}
          <button
            type="button"
            data-testid="prm-chat-retry"
            disabled={props.retryBlocked}
            onClick={props.onRetry}
          >
            Retry
          </button>
        </div>
      )}
    </li>
  );
}

function ProposalCard(props: {
  proposal: ChatProposal;
  readOnly: boolean;
  onReplace(p: ChatProposal): Promise<void>;
}) {
  const p = props.proposal;
  const [copied, setCopied] = useState(false);
  return (
    <div className="prm-chat-proposal" data-testid="prm-chat-proposal" data-record-id={p.id}>
      <pre className="prm-chat-code">
        <code>{p.text}</code>
      </pre>
      <div className="prm-chat-actions">
        <button
          type="button"
          className="primary"
          data-testid="prm-chat-proposal-replace"
          disabled={props.readOnly}
          onClick={() => void props.onReplace(p)}
        >
          Replace prompt
        </button>
        <button
          type="button"
          data-testid="prm-chat-proposal-copy"
          onClick={() =>
            void navigator.clipboard?.writeText(p.text).then(
              () => setCopied(true),
              () => undefined,
            )
          }
        >
          {copied ? 'Copied' : 'Copy'}
        </button>
        {p.inserted_at && (
          <span className="muted small">
            Inserted {new Date(p.inserted_at).toLocaleTimeString()}
          </span>
        )}
      </div>
    </div>
  );
}
