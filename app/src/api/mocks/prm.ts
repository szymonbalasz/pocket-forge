// Mock of the PRM helper commands (prompt-helper §9, slice 1 core) for Vitest and Playwright:
// `prm_helpers_list` / `_save` / `_reset` with the starter set (Appendix A, all off, #60) and
// the core's rules (FR-PRM-01..03, R78), and `prm_compose_preview` with the core's compose and
// dedupe (FR-PRM-06..08). Save and reset emit `prm:helpers-changed`.
//
// Like the other mocks it is **self-contained** (Playwright serialises it with `toString()`):
// nothing outside the function body is referenced. Never imported by app code.
import type { PromptHelper } from '../types';

export interface PrmMockSeed {
  /** The stored list (settings order); absent = the starter set. */
  helpers?: PromptHelper[];
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
}
