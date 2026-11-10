// The prompt chat window (prompt-helper §7, FR-PRM-20..29, PRM slice 6): its own webview
// (label `prompt-chat`, R91), bound to the asset project open in the main window. Header
// (project · host · backend chip), transcript with proposal cards (Replace + Copy, #60), the
// context strip, the composer (Send / Stop) and the usage footer. Every state of §7: consent,
// off, backend errors with their fix and Recheck, no project, no prompt block, running,
// errors with Retry, offline, read-only, fake. It follows the main window through
// `prm:chat-context` (project and host changes, BL-41), refreshes the strip on
// `prm:context-published` (R103) and re-reads on focus.
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
  fixWorthShowing,
  PRIVACY_NOTICE,
  questionOf,
  replySegments,
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

/** Night review 9: a turn of another project still holds the one chat slot (it finishes after
 *  a project switch, R103), so the core refuses a send with `turn_active`. A turn seen only
 *  through that refusal is {@link SOME_TURN}. With no word from it for this long (a missed
 *  end event), Send stops waiting. */
export const ELSEWHERE_IDLE_MS = 10_000;
const SOME_TURN = '?';
export const ELSEWHERE_REASON = 'A reply in another project is still finishing. Send waits for it.';

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
  /** FR-PRM-27 (code review 4): a send is on its way; Send, Retry and Resend wait. */
  const [sending, setSending] = useState(false);
  /** §11 (code review 1): the last send or reply couldn't be written to the thread file. */
  const [notSaved, setNotSaved] = useState(false);
  const sendingRef = useRef(false);
  const [includeImages, setIncludeImages] = useState(true);
  const [declined, setDeclined] = useState(false);
  /** L2: what Log in… did ("A console opened…"). */
  const [loginSaid, setLoginSaid] = useState<string | null>(null);
  /** L3: a failed turn's detail (stderr tail / message), from its event; never stored. */
  const [details, setDetails] = useState<Record<string, string>>({});
  const [preview, setPreview] = useState<ChatPreview | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [list, setList] = useState<HTMLOListElement | null>(null);
  /** M3: the transcript follows the newest turn unless the user scrolled up. */
  const stick = useRef(true);
  const deltas = useRef(new Map<string, string>());
  const ended = useRef(new Set<string>());
  /** Night review 9: turns running for any project (ours are filtered out when shown). */
  const [elsewhere, setElsewhere] = useState<ReadonlySet<string>>(new Set());
  const elsewhereTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const runningRef = useRef<string | null>(null);
  const noteElsewhere = useCallback((turnId: string | null, add: boolean) => {
    setElsewhere((s) => {
      if (add ? !turnId || s.has(turnId) : !s.has(SOME_TURN) && !(turnId && s.has(turnId)))
        return s;
      const next = new Set(s);
      if (add) next.add(turnId!);
      else {
        // Any turn's end frees the slot a refused send saw.
        next.delete(SOME_TURN);
        if (turnId) next.delete(turnId);
      }
      return next;
    });
    if (elsewhereTimer.current) clearTimeout(elsewhereTimer.current);
    elsewhereTimer.current = setTimeout(() => setElsewhere(new Set()), ELSEWHERE_IDLE_MS);
  }, []);
  useEffect(
    () => () => {
      if (elsewhereTimer.current) clearTimeout(elsewhereTimer.current);
    },
    [],
  );
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

  /** BL-43: numbers each thread load, so an older reply that arrives late never overwrites a
   *  newer one (only the latest load's reply is applied). */
  const threadSeq = useRef(0);
  const loadThread = useCallback(async (id: string | null) => {
    const seq = ++threadSeq.current;
    if (!id) return setView(null);
    try {
      const v = await api.chat.thread(id);
      if (seq === threadSeq.current && pidRef.current === id)
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
      setRefreshTick((n) => n + 1);
    };
    window.addEventListener('focus', focus);
    return () => window.removeEventListener('focus', focus);
  }, [loadStatus, loadProject]);

  useEffect(() => {
    // The previous project's running turn goes on after the switch and holds the slot.
    const carried = runningRef.current;
    if (carried && !ended.current.has(carried)) noteElsewhere(carried, true);
    runningRef.current = null;
    setLive(null);
    setProblem(null);
    setNotSaved(false);
    void loadThread(pid);
  }, [pid, loadThread, noteElsewhere]);

  // FR-PRM-26 / BL-42: what the next turn would send, refreshed when the host, the thread or
  // Include images changes, when the main window publishes context (R103), and on focus.
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
  }, [pid, host, includeImages, turnCount, refreshTick]);

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
        // Our own send's first delta can come before its answer: not another project's.
        if (!sendingRef.current) noteElsewhere(d.turn_id, true);
        setLive((l) => (l && l.turnId === d.turn_id ? { ...l, text: l.text + d.text } : l));
      }),
      onEvent('prm:chat-turn', (t) => {
        ended.current.add(t.turn_id);
        noteElsewhere(t.turn_id, false);
        setNotSaved(!t.saved);
        setLive((l) => (l && l.turnId === t.turn_id ? null : l));
        if (t.error?.code === 'offline') setOffline(true);
        const detail = t.error?.detail;
        if (detail) setDetails((d) => ({ ...d, [t.turn_id]: detail }));
        void loadThread(pidRef.current);
      }),
      // L2 / L6: detection, settings and consent changes are followed without a refocus.
      onEvent('prm:chat-status', (p) => setStatus((s) => (s ? { ...s, ...p } : s))),
      // BL-41: the main window opened, closed or switched project, or its visible tab changed.
      onEvent('prm:chat-context', (c) => {
        if (c.project_id !== pidRef.current) void loadProject();
        else void loadThread(c.project_id);
      }),
      // R103: a main-window edit (a publish) refreshes the next-turn strip.
      onEvent('prm:context-published', (c) => {
        if (c.project_id === pidRef.current) setRefreshTick((n) => n + 1);
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
  }, [loadThread, loadProject, noteElsewhere]);

  const send = async (text: string) => {
    // A second click before the first send answers is ignored (the ref is set at once).
    if (!pid || sendingRef.current) return;
    sendingRef.current = true;
    setSending(true);
    setProblem(null);
    try {
      await flushMainWindow();
      const out = await api.chat.send(pid, text, includeImages);
      setNotSaved(!out.saved);
      if (!ended.current.has(out.turn_id))
        setLive({ turnId: out.turn_id, text: deltas.current.get(out.turn_id) ?? '' });
      await loadThread(pid);
    } catch (e) {
      const p = problemOf(e);
      if (p.code === 'offline') setOffline(true);
      if (p.code === 'turn_active') {
        // Night review 9: not this project's turn (that one shows Stop): Send waits, saying why.
        noteElsewhere(SOME_TURN, true);
        await loadThread(pid);
      } else setProblem(p);
    } finally {
      sendingRef.current = false;
      setSending(false);
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
        {backendChip(status, true)}
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
            data-testid="prm-chat-enable"
            onClick={() => void emitEvent('prm:chat-enable', {}).catch(() => undefined)}
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
  runningRef.current = running;
  const ours = new Set(turns.map((t) => t.id));
  const busyElsewhere = !running && [...elsewhere].some((id) => !ours.has(id));
  const readOnly = !!project.read_only;
  const usage = usageLabel(turns, fake);
  // R110: each reason has its own test id (`prm-chat-blocked-<reason>`).
  const blocked: Blocked | null = !host
    ? { reason: 'no-host', text: 'Open the Generate panel to chat about this project.' }
    : offline
      ? { reason: 'offline', text: 'Offline: Recheck once the connection is back.' }
      : busyElsewhere
        ? { reason: 'elsewhere', text: ELSEWHERE_REASON }
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
            detail={details[t.id]}
            readOnly={readOnly}
            onReplace={replace}
            retryBlocked={offline || sending || !!running || busyElsewhere}
            onRetry={() => {
              const q = questionOf(turns, t.id);
              // M5: Retry respects the offline lock, as Send does.
              if (q && !offline && !running && !busyElsewhere) void send(q.text);
            }}
          />
        ))}
      </ol>
      {!readOnly && (notSaved || unsaved.size > 0) && (
        <p className="prm-chat-warn" role="status" data-testid="prm-chat-not-saved">
          Not saved: the chat file is locked. These turns are kept until the project closes.
        </p>
      )}
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
        sending={sending}
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

/** Why Send is disabled; `reason` names its test id (R110). */
interface Blocked {
  reason: 'no-host' | 'offline' | 'elsewhere';
  text: string;
}

function Composer(props: {
  disabledReason: Blocked | null;
  /** M5: offline after a turn: Recheck lifts the lock once detection succeeds. */
  onRecheck?: () => void;
  running: string | null;
  /** A send is on its way (FR-PRM-27). */
  sending: boolean;
  onSend(text: string): Promise<void>;
  onStop(): void;
}) {
  const [text, setText] = useState('');
  const canSend =
    !props.disabledReason && !props.running && !props.sending && text.trim().length > 0;
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
        {props.disabledReason && (
          <span
            className="muted small"
            data-testid={`prm-chat-blocked-${props.disabledReason.reason}`}
          >
            {props.disabledReason.text}
          </span>
        )}
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
  /** Kept in memory only (read-only project, §7 / R94; a locked thread file, §11). */
  unsaved: boolean;
  /** M5: offline: Retry and Resend wait for Recheck, as Send does; and while a turn is sent or
   *  runs (FR-PRM-27). */
  retryBlocked: boolean;
  /** L3: the error's detail while the window is open (FR-PRM-37: never written). */
  detail?: string;
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
      {replySegments(text, props.running ? 0 : (t.proposals ?? []).length).map((s, i) =>
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
            data-testid="prm-chat-resend"
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
          {fixWorthShowing(t.error.fix) && <p className="muted small">{t.error.fix}</p>}
          {props.detail && (
            <details className="small" data-testid="prm-chat-error-detail">
              <summary>Details</summary>
              <pre className="prm-chat-detail">{props.detail}</pre>
            </details>
          )}
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
