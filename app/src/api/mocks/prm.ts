// Mock of the PRM commands (prompt-helper §9) for Vitest and Playwright. Slice 1 core:
// `prm_helpers_list` / `_save` / `_reset` with the starter set (Appendix A, all off, #60) and
// the core's rules (FR-PRM-01..03, R78), and `prm_compose_preview` with the core's compose and
// dedupe (FR-PRM-06..08). Save and reset emit `prm:helpers-changed`. Slice 5 chat core: the
// `prm_chat_*` / `prm_context_*` commands over a fake backend like the core's (FR-PRM-39): the
// reply quotes the text and ends with a `prompt` block; deltas and the turn end are emitted
// after the send returns; Replace emits `prm:insert` for the turn's host.
//
// Like the other mocks it is **self-contained** (Playwright serialises it with `toString()`):
// nothing outside the function body is referenced. Never imported by app code.
import type { PromptHelper } from '../types';

export interface PrmMockSeed {
  /** The stored list (settings order); absent = the starter set. */
  helpers?: PromptHelper[];
  /** The prompt chat (slice 5): consent given and on; the fake case (FR-PRM-39). */
  chat?: { consent?: boolean; case?: string };
}

export function installPrmMock(seed: PrmMockSeed = {}): void {
  type Rec = Record<string, unknown>;
  type Kind = 'map' | 'token' | 'item';
  type H = PromptHelper;

  const w = window as unknown as Rec;
  const handlers = (w.__pfMockHandlers ??= {}) as Record<string, (input: Rec) => unknown>;
  const fail = (code: string, message: string, field?: string, detail?: unknown) =>
    Promise.reject({ code, message, ...(field ? { field } : {}), ...(detail ? { detail } : {}) });
  const emit = (event: string, payload: unknown) => {
    const m = w.__pfMock as { emit?: (e: string, p: unknown) => void } | undefined;
    if (m?.emit) m.emit(event, payload);
    else
      (w.__pfVitestCore as { emit?: (e: string, p: unknown) => void } | undefined)?.emit?.(
        event,
        payload,
      );
  };

  const ALL: Kind[] = ['map', 'token', 'item'];
  const starter: H[] = (
    [
      ['top-down-map', ['map'], 'Top-down battlemap', 'Top-down battlemap view.'],
      [
        'match-style',
        ['map'],
        'Match surrounding style',
        'Match the existing art style, palette, line weight and lighting.',
      ],
      ['no-people', ['map'], 'No people or tokens', 'No people, creatures or tokens.'],
      ['no-grid-text', ['map'], 'No grid or text', 'No grid lines, labels or text.'],
      [
        'token-single',
        ['token'],
        'Single creature',
        'A single creature, centred and fully in frame, seen from above.',
      ],
      ['token-plain-bg', ['token'], 'Plain background', 'Plain, uncluttered background.'],
      ['item-single', ['item'], 'Single item', 'A single item, centred and fully in frame.'],
      [
        'item-plain-bg',
        ['item'],
        'Plain background',
        'Plain neutral background, soft even lighting.',
      ],
      ['no-text', ['token', 'item'], 'No text', 'No text, frame, border or watermark.'],
    ] as [string, Kind[], string, string][]
  ).map(([slug, kinds, label, text]) => ({
    id: `builtin:${slug}`,
    label,
    text,
    kinds,
    default_on: false,
    builtin: true,
    hidden: false,
    modified: false,
  }));
  const starterOf = (id: string) => starter.find((s) => s.id === id);
  const canonical = (kinds: Kind[]) => ALL.filter((k) => kinds.includes(k));
  const withModified = (h: H): H => {
    const s = starterOf(h.id);
    const modified =
      !!s &&
      (s.label !== h.label ||
        s.text !== h.text ||
        s.kinds.join() !== h.kinds.join() ||
        s.default_on !== h.default_on);
    return { ...h, builtin: !!s, modified };
  };

  let list: H[] = (seed.helpers ?? starter).map((h) => ({ ...h }));
  const out = () => list.map((h) => ({ ...h, kinds: [...h.kinds] }));
  const store = (next: H[]) => {
    list = next.map(withModified);
    emit('prm:helpers-changed', out());
    return out();
  };
  let nextUlid = 1;
  const newId = () => `01JPRMMOCK${String(nextUlid++).padStart(16, '0')}`;

  handlers.prm_helpers_list = (input) => {
    const kind = input.kind as Kind | undefined;
    return out().filter((h) => !kind || h.kinds.includes(kind));
  };

  handlers.prm_helpers_save = (input) => {
    const rows = (input.helpers ?? []) as Rec[];
    if (rows.length > 60)
      return fail('too_many', 'at most 60 prompt helpers', 'helpers', { max: 60 });
    const seen = new Set<string>();
    for (const [i, r] of rows.entries()) {
      const id = r.id as string | undefined;
      if (id !== undefined) {
        if (id.startsWith('builtin:') ? !starterOf(id) : !/^[0-9A-Z]{26}$/.test(id))
          return fail(
            'invalid_input',
            'not a known built-in or a user helper id',
            `helpers[${i}].id`,
          );
        if (seen.has(id))
          return fail('invalid_input', 'the same helper is listed twice', `helpers[${i}].id`);
        seen.add(id);
      }
      const label = String(r.label ?? '').trim();
      const text = String(r.text ?? '').trim();
      if (label.length < 1 || [...label].length > 40)
        return fail('invalid_input', 'a helper label is 1 to 40 characters', `helpers[${i}].label`);
      if (text.length < 1 || [...text].length > 500)
        return fail('invalid_input', 'a helper text is 1 to 500 characters', `helpers[${i}].text`);
      if (!Array.isArray(r.kinds) || r.kinds.length === 0)
        return fail(
          'invalid_input',
          'a helper needs at least one asset type',
          `helpers[${i}].kinds`,
        );
    }
    const gone = starter.find((s) => !seen.has(s.id));
    if (gone)
      return fail(
        'invalid_input',
        `the built-in helper "${gone.label}" can't be deleted; hide it instead`,
        'helpers',
        { id: gone.id },
      );
    return store(
      rows.map((r) => ({
        id: (r.id as string | undefined) ?? newId(),
        label: String(r.label).trim(),
        text: String(r.text).trim(),
        kinds: canonical(r.kinds as Kind[]),
        default_on: !!r.default_on,
        hidden: !!r.hidden,
        builtin: false,
        modified: false,
      })),
    );
  };

  handlers.prm_helpers_reset = (input) => {
    if (input.all === true)
      return store([...starter.map((s) => ({ ...s })), ...list.filter((h) => !h.builtin)]);
    const id = String(input.id ?? '');
    const at = list.findIndex((h) => h.id === id);
    if (at < 0) return fail('not_found', 'no such prompt helper', 'id');
    const s = starterOf(id);
    if (!s) return fail('invalid_input', 'only a built-in helper can be reset', 'id');
    const next = [...list];
    next[at] = { ...s };
    return store(next);
  };

  // FR-PRM-06/07: the core's compose (pf-core `prompt::compose`).
  const normalise = (t: string) => {
    let n = t.split(/\s+/).filter(Boolean).join(' ').toLowerCase();
    if (n.endsWith('.')) n = n.slice(0, -1).trimEnd();
    return n;
  };
  const isWord = (c: string | undefined) => !!c && /[\p{L}\p{N}]/u.test(c);
  const containsPhrase = (hay: string, needle: string) => {
    if (!needle) return false;
    for (let at = hay.indexOf(needle); at >= 0; at = hay.indexOf(needle, at + 1))
      if (!isWord(hay[at - 1]) && !isWord(hay[at + needle.length])) return true;
    return false;
  };
  handlers.prm_compose_preview = (input) => {
    const kind = input.kind as Kind;
    if (!ALL.includes(kind)) return fail('invalid_input', 'invalid input: kind', 'input');
    const typed = String(input.typed_prompt ?? '').trimEnd();
    const ids = input.helper_ids as string[] | null | undefined;
    const blend = typeof input.blend_instruction === 'string' ? input.blend_instruction.trim() : '';
    const applies = (h: H) => !h.hidden && h.kinds.includes(kind);
    const ticked = ids ?? list.filter((h) => applies(h) && h.default_on).map((h) => h.id);
    const blendNorm = normalise(blend);
    const sent: H[] = [];
    const skipped: { id: string; reason: string }[] = [];
    const earlier: string[] = [];
    for (const h of list.filter((h) => applies(h) && ticked.includes(h.id))) {
      const n = normalise(h.text);
      if (blend && containsPhrase(blendNorm, n))
        skipped.push({ id: h.id, reason: 'in_blend_instruction' });
      else if (earlier.includes(n)) skipped.push({ id: h.id, reason: 'duplicate' });
      else sent.push(h);
      earlier.push(n);
    }
    // As pf-core: a typed prompt with nothing added is kept byte for byte (FR-RGN-09).
    const alone = !sent.length && !blend && typed;
    const shown = alone ? String(input.typed_prompt) : typed;
    if (alone)
      return {
        text: shown,
        parts: [{ source: 'typed', text: shown }],
        skipped,
        chars: [...shown].length,
      };
    const parts = [
      ...(typed ? [{ source: 'typed', text: typed }] : []),
      ...sent.map((h) => ({ source: 'helper', id: h.id, text: h.text.trim() })),
      ...(blend ? [{ source: 'blend', text: blend }] : []),
    ];
    const blocks = [typed, sent.map((h) => h.text.trim()).join(' '), blend].filter((b) => b !== '');
    const text = blocks.join('\n\n');
    return { text, parts, skipped, chars: [...text].length };
  };
  // Hooks for the host mocks (RGN; GEN in its slice 6), as pf-core's `compose` / `retry_ticks`.
  /** FR-PRM-09: the helpers a submit sends (`{id, label, text}` in order). */
  handlers.__prm_sent = async (input) => {
    const p = (await handlers.prm_compose_preview!(input)) as {
      parts: { source: string; id?: string; text: string }[];
    };
    return p.parts
      .filter((x) => x.source === 'helper')
      .map((x) => ({
        id: x.id,
        label: list.find((h) => h.id === x.id)?.label ?? '',
        text: x.text,
      }));
  };
  /** FR-PRM-10 (R78): the job's helpers ticked again; changed texts badged; a deleted, hidden
   *  or wrong-kind helper is missing. `prompt_retry` only when there is something to badge. */
  handlers.__prm_retry = (input) => {
    const kind = input.kind as Kind;
    const recorded = (input.recorded ?? []) as { id: string; label: string; text: string }[];
    const ticks: string[] = [];
    const changed: string[] = [];
    const missing: typeof recorded = [];
    for (const r of recorded) {
      const h = list.find((x) => x.id === r.id && !x.hidden && x.kinds.includes(kind));
      if (!h) missing.push(r);
      else {
        if (h.text !== r.text) changed.push(h.id);
        ticks.push(h.id);
      }
    }
    return changed.length || missing.length
      ? { ticks, prompt_retry: { since: input.since, changed, missing } }
      : { ticks };
  };

  // ── the prompt chat (slice 5) ──────────────────────────────────────────────────────────
  type Host = 'rgn' | 'gen';
  type Turn = Rec & { id: string; role: string; host: Host; state: string; text: string };
  const chat = {
    consent: seed.chat?.consent ?? false,
    enabled: seed.chat?.consent ?? false,
    fakeCase: seed.chat?.case ?? 'success',
    claudePath: null as string | null,
    windowOpens: 0,
    seq: 0,
    contexts: new Map<string, { seq: number; input: unknown }>(),
    threads: new Map<string, { id: string; turns: Turn[]; updated_at: string }>(),
    running: null as null | { turnId: string; cancel: () => void },
    session: null as null | string,
    /** §7 / R94: turns sent on a read-only project (kept in memory only). */
    unsaved: [] as string[],
    ids: 0,
  };
  const chatId = () => `01CHAT${String(++chat.ids).padStart(20, '0')}`;
  const now = () => new Date().toISOString();
  const statusOf = () => {
    const off = !chat.consent || !chat.enabled;
    const errs: Record<string, [string, string]> = {
      not_installed: [
        "Claude Code isn't installed, or isn't on PATH.",
        'Install Claude Code, or set its path in Settings → Prompt chat.',
      ],
      not_logged_in: [
        "The prompt chat isn't logged in to Claude.",
        'Log in… in Settings → Prompt chat, then Recheck.',
      ],
      isolation_unavailable: [
        "Prompt chat can't run Claude Code without your own Claude Code settings, so it stays off.",
        'Update Claude Code, then Recheck.',
      ],
      version_old: ['Claude Code 2.1.200 is too old (needs 2.1.319).', 'Run `claude update`.'],
    };
    const e = errs[chat.fakeCase];
    const backend = off
      ? { state: 'not_checked' }
      : e
        ? {
            state: 'error',
            code: chat.fakeCase === 'version_old' ? 'version_unsupported' : chat.fakeCase,
            message: e[0],
            fix: e[1],
          }
        : { state: 'ok', version: '2.1.319', auth_method: 'fake', plan: null, notional: false };
    return {
      enabled: chat.enabled,
      consent: chat.consent,
      notice_version: 1,
      mode: 'fake',
      backend,
      fake_case: chat.fakeCase,
      login_command: null,
      claude_path: chat.claudePath,
    };
  };
  const hostLabel = (h: Host) => (h === 'rgn' ? 'Region edit' : 'New image');
  handlers.prm_chat_status = () => statusOf();
  handlers.prm_chat_consent = (input) => {
    if (input.accept) {
      chat.consent = true;
      chat.enabled = true;
    }
    return statusOf();
  };
  handlers.prm_chat_settings_put = (input) => {
    if (typeof input.enabled === 'boolean') chat.enabled = input.enabled;
    if ('claude_path' in input) chat.claudePath = (input.claude_path as string | null) ?? null;
    if (!chat.enabled) chat.session = null;
    return statusOf();
  };
  handlers.prm_chat_logout = () =>
    chat.consent ? statusOf() : fail('consent_required', 'accept the privacy notice first');
  handlers.prm_chat_session_end = () => {
    chat.running?.cancel();
    chat.session = null;
    return {};
  };
  // Tauri-layer commands (R91): e2e replaces `prm_window_open` with a popup page.
  handlers.prm_window_open = () => {
    chat.windowOpens += 1;
    return null;
  };
  handlers.prm_chat_login = () =>
    chat.consent ? null : fail('consent_required', 'accept the privacy notice first');
  handlers.prm_chat_fake_set_case = (input) => {
    chat.fakeCase = String(input.case);
    return {};
  };
  const currentHost = (pid: string): Host | null => {
    let host: Host | null = null;
    let best = -1;
    for (const h of ['rgn', 'gen'] as Host[]) {
      const c = chat.contexts.get(`${pid}|${h}`);
      if (c && c.seq > best) {
        best = c.seq;
        host = h;
      }
    }
    return host;
  };
  // BL-41: a host change is announced, as the core does.
  const announce = (pid: string, before: Host | null) => {
    const after = currentHost(pid);
    if (after !== before) emit('prm:chat-context', { project_id: pid, host: after });
  };
  handlers.prm_context_publish = (input) => {
    const pid = String(input.project_id);
    const before = currentHost(pid);
    chat.contexts.set(`${pid}|${String(input.host)}`, { seq: ++chat.seq, input: input.input });
    announce(pid, before);
    return {};
  };
  // FR-PRM-26 / BL-42: what the next turn would send.
  handlers.prm_chat_preview = (input) => {
    const host = currentHost(String(input.project_id));
    const withImages = input.include_images !== false;
    return {
      host,
      summary: !host
        ? ''
        : withImages
          ? host === 'rgn'
            ? 'crop 556×456 + mask · prompt · fake model'
            : '6 images · prompt · fake model'
          : 'text only · prompt · fake model',
      images:
        host === 'rgn' && withImages
          ? [
              {
                id: 'rgn:overlay',
                label: 'region crop with the mask',
                w: 556,
                h: 456,
                thumb: 'data:image/gif;base64,R0lGODlhAQABAAAAACw=',
                already_sent: false,
              },
            ]
          : [],
      dropped: [],
    };
  };
  handlers.prm_context_clear = (input) => {
    const pid = String(input.project_id);
    const before = currentHost(pid);
    chat.contexts.delete(`${pid}|${String(input.host)}`);
    announce(pid, before);
    return {};
  };
  handlers.prm_chat_thread_get = (input) => {
    const t = chat.threads.get(String(input.project_id));
    // A copy, as the real IPC would return (the window compares views).
    return JSON.parse(
      JSON.stringify({
        thread: t
          ? {
              schema_version: 1,
              id: t.id,
              created_at: t.updated_at,
              updated_at: t.updated_at,
              backend: 'fake',
              turns: t.turns,
            }
          : null,
        running_turn_id: chat.running?.turnId ?? null,
        corrupt_set_aside: false,
        unsaved_turn_ids: chat.unsaved,
        fresh_session_next: chat.session !== String(input.project_id),
        current_host: (() => {
          let host: Host | null = null;
          let best = -1;
          for (const h of ['rgn', 'gen'] as Host[]) {
            const c = chat.contexts.get(`${String(input.project_id)}|${h}`);
            if (c && c.seq > best) {
              best = c.seq;
              host = h;
            }
          }
          return host;
        })(),
      }),
    );
  };
  handlers.prm_chat_send = (input) => {
    const pid = String(input.project_id);
    const text = String(input.text ?? '').trim();
    if (!text || String(input.text).length > 8000)
      return fail('invalid_input', 'a message is 1–8000 characters', 'text');
    if (!chat.consent) return fail('consent_required', 'accept the privacy notice first');
    if (!chat.enabled) return fail('chat_disabled', 'the prompt chat is off in Settings');
    const st = statusOf().backend as Rec;
    if (st.state === 'error') return fail(String(st.code), String(st.message), undefined, st);
    if (chat.running) return fail('turn_active', 'wait for the current reply, or stop it');
    let host: Host | null = null;
    let best = -1;
    for (const h of ['rgn', 'gen'] as Host[]) {
      const c = chat.contexts.get(`${pid}|${h}`);
      if (c && c.seq > best) {
        best = c.seq;
        host = h;
      }
    }
    if (!host) return fail('no_prompt_block', 'open the Generate panel to chat about this project');
    const fresh = chat.session !== pid;
    chat.session = pid;
    const t = chat.threads.get(pid) ?? { id: chatId(), turns: [], updated_at: now() };
    chat.threads.set(pid, t);
    const userId = chatId();
    const turnId = chatId();
    const readOnly = !!(handlers.project_get?.({}) as Rec | null)?.read_only;
    if (readOnly) chat.unsaved.push(userId, turnId);
    const summary =
      host === 'rgn'
        ? 'crop 556×456 + mask · prompt · fake model'
        : '6 images · prompt · fake model';
    const dropped = host === 'gen' ? ['teal.png', 'yellow.png', 'red.png'] : [];
    t.turns.push({
      id: userId,
      at: now(),
      role: 'user',
      host,
      text,
      state: 'pending',
      context: {
        summary,
        images: [],
        dropped,
        final_prompt: text,
        model_id: 'fake',
        controls: {},
      },
      ...(fresh ? { fresh_session: true } : {}),
    });
    const reply: Turn = {
      id: turnId,
      at: now(),
      role: 'assistant',
      host,
      text: '',
      state: 'pending',
    };
    t.turns.push(reply);
    const blocks = chat.fakeCase === 'two_proposals' ? 2 : chat.fakeCase === 'no_proposal' ? 0 : 1;
    let body = `Fake chat: no model was called.\nYou wrote: "${text}"\nHost: ${hostLabel(host)}\n`;
    const props = [text, `${text}, seen from directly above`].slice(0, blocks);
    for (const p of props) body += `\n\`\`\`prompt\n${p}\n\`\`\`\n`;
    const pieces = chat.fakeCase.startsWith('slow')
      ? (body.match(/[\s\S]{1,24}/g) ?? [body])
      : [body];
    let canceled = false;
    const finish = (state: string, extra: Rec = {}) => {
      reply.state = state;
      if (extra.error) reply.error = extra.error;
      t.turns.find((x) => x.id === userId)!.state = 'done';
      t.updated_at = now();
      chat.running = null;
      if (state !== 'done') chat.session = null;
      emit('prm:chat-turn', { turn_id: turnId, state, saved: !readOnly, ...extra });
    };
    chat.running = {
      turnId,
      cancel: () => {
        canceled = true;
      },
    };
    let i = 0;
    const step = () => {
      if (canceled) return finish('canceled');
      if (chat.fakeCase === 'usage_limit')
        return finish('failed', {
          error: {
            code: 'usage_limit',
            message: "Your Claude plan's usage limit is reached.",
            fix: "You've hit your limit · resets 3pm",
          },
        });
      if (chat.fakeCase === 'offline')
        return finish('failed', {
          error: {
            code: 'offline',
            message: "Can't reach Anthropic.",
            fix: 'Check the connection, then Recheck.',
            detail: 'API Error: Connection error.',
          },
        });
      if (i < pieces.length) {
        const piece = pieces[i++]!;
        reply.text += piece;
        emit('prm:chat-delta', { turn_id: turnId, text: piece });
        setTimeout(step, chat.fakeCase.startsWith('slow') ? 200 : 0);
        return;
      }
      const proposals = props.map((p) => ({ id: chatId(), text: p }));
      reply.proposals = proposals;
      reply.usage = { input_tokens: 100, output_tokens: 40, cost_usd: 0.0009, notional: false };
      finish('done', { usage: reply.usage, ...(proposals.length ? { proposals } : {}) });
    };
    setTimeout(step, 0);
    return {
      turn_id: turnId,
      user_turn_id: userId,
      thread_id: t.id,
      host,
      context_summary: summary,
      dropped,
      fresh_session: fresh,
      saved: !readOnly,
    };
  };
  handlers.prm_chat_cancel = (input) => {
    const running = chat.running;
    if (!running || running.turnId !== input.turn_id)
      return fail('not_found', "that turn isn't running");
    running.cancel();
    return {};
  };
  handlers.prm_chat_insert = (input) => {
    const pid = String(input.project_id);
    const t = chat.threads.get(pid);
    for (const turn of t?.turns ?? []) {
      const p = ((turn.proposals ?? []) as Rec[]).find((x) => x.id === input.proposal_id);
      if (!p) continue;
      if (!chat.contexts.has(`${pid}|${turn.host}`))
        return fail('no_prompt_block', `switch to the ${hostLabel(turn.host)} tab to insert`);
      p.inserted_at = now();
      emit('prm:insert', { project_id: pid, host: turn.host, proposal_id: p.id, text: p.text });
      return { proposal_id: p.id, host: turn.host, inserted_at: p.inserted_at };
    }
    return fail('not_found', 'no such proposal');
  };
}
