// Mock of every `rgn_*` command (RGN §9: names, payloads, errors), plus the PRV model calls and
// `project_restore_revision` that the Generate panel needs. Used by Vitest (through
// `installFakeTauri`) and by Playwright (`e2e/rgn-mock.ts` passes this function to
// `page.addInitScript`). It is therefore **self-contained**: nothing outside the function body is
// referenced, because Playwright serialises it with `toString()`. Never imported by app code.
//
// It registers handlers in `window.__pfMockHandlers`, keeps snapshot fields (candidates, layers,
// draft, read_only) in `window.__pfMockSnapshot`, emits events through `window.__pfMock.emit`,
// and exposes `window.__pfRgnMock` so a test can drive the job (advance, fail, …).
import type { Capabilities } from '../types';

export interface RgnMockSeed {
  image: { w: number; h: number };
  models: { id: string; name: string }[];
  modelsStale?: boolean;
  capabilities: Record<string, Capabilities>;
  spend: { cap: number | null; threshold: number | null };
  /** Cost per candidate (×2 with `hd`); `'unknown'` → the estimate is unknown. */
  costPerSample?: number | 'unknown';
  unit?: string;
  maskId?: string | null;
  readOnly?: boolean;
  /** `rgn_job_cancel` answer while the job is cancellable. */
  cancel?: 'canceled' | 'not_cancellable';
  /** Commands that fail with this error once called (code → {message}). */
  failures?: Record<string, { code: string; message: string; field?: string }>;
  /** #55: the stored blend instruction text (absent = the default). */
  blendInstruction?: string | null;
  /** #55: the open project's kind (Maps add the view sentence); default `map`. */
  kind?: 'map' | 'token' | 'item';
}

export function installRgnMock(seed: RgnMockSeed): void {
  type Rec = Record<string, unknown>;
  type Rect = { x: number; y: number; w: number; h: number };
  type Ctl = Capabilities['controls'][number];
  interface Job {
    id: string;
    seq: number;
    state: string;
    draft: Rec & { region: { inner: Rect; margin: number }; controls: Rec; final_prompt: string };
    outer: Rect;
    cost: number | 'unknown';
    n: number | null;
    dir: string | null;
    slug: string | null;
    resolved: boolean;
    error: Rec | null;
    progress: number | null;
    /** The job's `references[]` (FR-RGN-43), from the REF mock's prepare hook. */
    refs: Rec[];
  }
  interface Cand {
    id: string;
    job_id: string;
    index: number;
    file: string;
    status: string;
    starred: boolean;
    trash_id: string | null;
    width: number;
    height: number;
    aspect_warning?: boolean;
  }

  const w = window as unknown as Rec;
  const handlers = (w.__pfMockHandlers ??= {}) as Record<string, (input: Rec) => unknown>;
  const snap = (w.__pfMockSnapshot ??= {}) as Rec;
  const unit = seed.unit ?? 'CU';
  const original = {
    id: '01JLAYER00000000000000000',
    kind: 'original',
    file: null,
    cx: seed.image.w / 2,
    cy: seed.image.h / 2,
    scale: 1,
    rotation: 0,
    opacity: 1,
    visible: true,
  };
  const s = {
    spend: 0,
    cap: seed.spend.cap,
    threshold: seed.spend.threshold,
    jobs: [] as Job[],
    cands: [] as Cand[],
    layers: [original] as Rec[],
    revs: [] as { id: string; seq: number; layers: Rec[]; reason: string; label?: string }[],
    nextId: 1,
    estimates: 0,
    cancel: seed.cancel ?? 'canceled',
    failures: { ...(seed.failures ?? {}) },
    connectivity: 'ok',
    // #55 (pf-core `region_edit::instruction`).
    blendDefault:
      'Keep everything near the edges of the edited area consistent with the surrounding map: ' +
      'continue the existing walls, floor tiles, paths, lighting, scale and line style across ' +
      'the boundary so there is no visible seam. Place the new content toward the centre of the ' +
      'edited area.',
    blendText: seed.blendInstruction ?? null,
  };
  /** `rgn_blend_instruction_get` / `_set` output. */
  const blendOut = () => {
    const text = s.blendText ?? s.blendDefault;
    const t = text.trim();
    const kind = seed.kind ?? 'map';
    return {
      text,
      default_text: s.blendDefault,
      is_default: s.blendText === null,
      for_project: t ? (kind === 'map' ? `${t} Top-down battlemap view.` : t) : '',
    };
  };
  snap.read_only = seed.readOnly ? 'newer_version' : null;
  snap.draft = {
    final_prompt: '',
    model_id: null,
    controls: {},
    seed: null,
    region: null,
    mask_id: seed.maskId === undefined ? '01JMASK000000000000000000' : seed.maskId,
    updated_at: '2026-11-07T11:00:00.000Z',
  };

  const id = (prefix: string) => `${prefix}${String(s.nextId++).padStart(4, '0')}`;
  const fail = (code: string, message: string, field?: string, detail?: unknown) =>
    Promise.reject({ code, message, ...(field ? { field } : {}), ...(detail ? { detail } : {}) });
  const emit = (event: string, payload: unknown) =>
    (w.__pfMock as { emit?: (e: string, p: unknown) => void } | undefined)?.emit?.(event, payload);
  const sync = () => {
    snap.candidates = s.cands.map((c) => ({ ...c }));
    snap.layers = s.layers.map((l) => ({ ...l }));
    // Revisions: only once the mock made one (mock-core owns the project's own until then).
    if (s.revs.length) {
      snap.revisions = s.revs.map(({ id, seq, reason, label }) => ({
        id,
        seq,
        created_at: '2026-11-07T11:00:00.000Z',
        reason,
        ...(label ? { label } : {}),
      }));
      snap.head_revision = s.revs.at(-1)!.id;
    } else snap.head_revision ??= 'r0';
  };
  const committed = () => {
    sync();
    emit('project:committed', { seq: s.nextId });
  };
  const caps = (modelId: unknown): Capabilities | undefined => seed.capabilities[String(modelId)];
  const terminal = (st: string) => ['failed', 'canceled', 'lost', 'closed'].includes(st);
  const active = () => s.jobs.find((j) => !terminal(j.state) && j.state !== 'fetched');
  const newest = () => s.jobs.at(-1);

  // FR-PRV-30 `evaluate`, the rules in order (hidden, allowed, required) + value checks.
  const evaluate = (c: Capabilities, values: Rec) => {
    const eff = new Map<string, { visible: boolean; allowed: unknown[]; required: boolean }>();
    for (const ctl of c.controls)
      eff.set(ctl.name, { visible: true, allowed: [...ctl.allowed], required: ctl.required });
    const same = (a: unknown, b: unknown) =>
      typeof a === 'number' && typeof b === 'number' ? a === b : a === b;
    for (const r of c.rules) {
      const v = values[r.if.control] ?? c.controls.find((x) => x.name === r.if.control)?.default;
      const hit =
        r.if.op === 'eq'
          ? same(v, r.if.value)
          : Array.isArray(r.if.value) && r.if.value.some((x) => same(x, v));
      const t = eff.get(r.then.control);
      if (!hit || !t) continue;
      if (r.then.hidden) t.visible = false;
      if (r.then.allowed) t.allowed = [...r.then.allowed];
      if (r.then.required !== undefined) t.required = r.then.required;
    }
    return c.controls.map((ctl: Ctl) => {
      const e = eff.get(ctl.name)!;
      const v = values[ctl.name];
      const out: Rec = {
        name: ctl.name,
        visible: e.visible,
        allowed: e.allowed,
        required: e.required,
      };
      if (!e.visible || ['image', 'mask'].includes(ctl.type)) return out;
      const empty = v === undefined || v === null || v === '';
      if (empty) {
        if (e.required) out.error = { code: 'required', message: `${ctl.name} is required` };
        return out;
      }
      if (ctl.type === 'int' || ctl.type === 'float' || ctl.type === 'seed') {
        if (typeof v !== 'number' || (ctl.type !== 'float' && !Number.isInteger(v)))
          out.error = { code: 'wrong_type', message: 'expected a number' };
        else if ((ctl.min !== null && v < ctl.min) || (ctl.max !== null && v > ctl.max))
          out.error = {
            code: 'out_of_range',
            message: `must be between ${ctl.min} and ${ctl.max}`,
          };
      } else if (ctl.type === 'bool' && typeof v !== 'boolean') {
        out.error = { code: 'wrong_type', message: 'expected true or false' };
      }
      if (!out.error && e.allowed.length && !e.allowed.some((a) => same(a, v)))
        out.error = { code: 'not_allowed', message: 'not one of the allowed values' };
      return out;
    }) as { name: string; visible: boolean; error?: { code: string; message: string } }[];
  };

  // `rgn_controls_evaluate` as pf-core does it: evaluate, reset rejected values to their
  // defaults, drop unknown names, evaluate again (FR-RGN-11/33).
  const evaluateControls = (c: Capabilities, given: Rec) => {
    const values: Rec = {};
    for (const [k, v] of Object.entries(given)) if (v !== null && v !== undefined) values[k] = v;
    const unknown = Object.keys(values).filter((k) => !c.controls.some((x) => x.name === k));
    const reset = evaluate(c, values)
      .filter((e) => e.error && e.error.code !== 'required')
      .map((e) => e.name);
    for (const k of [...unknown, ...reset]) delete values[k];
    const second = evaluate(c, values);
    const required_missing: string[] = [];
    const controls = c.controls
      .filter((x) => !['source', 'mask', 'reference'].includes(x.role))
      .map((x) => {
        const e = second.find((y) => y.name === x.name)! as unknown as Rec & {
          visible: boolean;
          allowed: unknown[];
          required: boolean;
          error?: { code: string; message: string };
        };
        let value: unknown = values[x.name] ?? x.default ?? null;
        let wasReset = reset.includes(x.name);
        if (e.visible && value !== null && e.allowed.length && !e.allowed.includes(value)) {
          value = e.allowed[0] ?? null;
          wasReset = true;
        }
        const required = e.required || (x.role === 'prompt' && c.prompt_required);
        let error = e.error ? { control: x.name, ...e.error } : undefined;
        if (e.visible && required && x.role === 'prompt' && !String(value ?? '').trim() && !error)
          error = { control: x.name, code: 'required', message: `${x.name} is required` };
        if (e.visible && error?.code === 'required') required_missing.push(x.name);
        return {
          name: x.name,
          type: x.type,
          role: x.role,
          min: x.min,
          max: x.max,
          step: x.step,
          default: x.default,
          cost_affecting: x.cost_affecting,
          visible: e.visible,
          allowed: e.allowed,
          required,
          value: e.visible ? value : null,
          reset: wasReset,
          ...(e.visible && error ? { error } : {}),
        };
      });
    return {
      model_id: c.model_id,
      controls,
      required_missing,
      unknown,
      warnings: c.warnings,
      prompt_required: c.prompt_required,
      seed: c.seed,
    };
  };

  // FR-RGN-01/02/05: snap, clamp, margin, fit to input_px (simplified: grow to a multiple when
  // there is room, else resample to the nearest valid size).
  const fit = (input: Rec) => {
    const inner = input.inner as Rect;
    const margin = input.margin === undefined ? 128 : Number(input.margin);
    const { w: iw, h: ih } = seed.image;
    const x0 = Math.max(0, Math.round(inner.x));
    const y0 = Math.max(0, Math.round(inner.y));
    const x1 = Math.min(iw, Math.round(inner.x + inner.w));
    const y1 = Math.min(ih, Math.round(inner.y + inner.h));
    if (x1 <= x0 || y1 <= y0) return fail('out_of_bounds', 'The region is outside the image.');
    if (x1 - x0 < 16 || y1 - y0 < 16)
      return fail('region_too_small', 'A region is at least 16 × 16 px.');
    const r = { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
    const outer = {
      x: Math.max(0, r.x - margin),
      y: Math.max(0, r.y - margin),
      w: 0,
      h: 0,
    };
    outer.w = Math.min(iw, r.x + r.w + margin) - outer.x;
    outer.h = Math.min(ih, r.y + r.h + margin) - outer.y;
    if (outer.w > 4096 || outer.h > 4096)
      return fail('region_too_large', 'The region plus its context margin is over 4096 px.');
    let sent_w = outer.w;
    let sent_h = outer.h;
    if (input.model_id) {
      const c = caps(input.model_id);
      if (!c) return fail('unknown_model', `unknown model ${String(input.model_id)}`);
      const { max, multiple_of: m } = c.input_px;
      const longest = Math.max(outer.w, outer.h);
      if (longest > max) {
        const k = max / longest;
        sent_w = Math.floor((outer.w * k) / m) * m;
        sent_h = Math.floor((outer.h * k) / m) * m;
      } else {
        sent_w = Math.round(outer.w / m) * m || m;
        sent_h = Math.round(outer.h / m) * m || m;
      }
    }
    return { inner: r, outer, margin, sent_w, sent_h, scale: sent_w / outer.w };
  };

  const countOf = (c: Capabilities, controls: Rec) => {
    const ctl = c.controls.find((x) => x.role === 'candidate_count');
    return Number(ctl ? (controls[ctl.name] ?? ctl.default ?? 1) : 1);
  };
  const costOf = (c: Capabilities, controls: Rec): number | 'unknown' => {
    if (seed.costPerSample === 'unknown') return 'unknown';
    return countOf(c, controls) * (seed.costPerSample ?? 1) * (controls.hd === true ? 2 : 1);
  };
  /** FR-RGN-41: what REF would send (the REF mock's `prepare_for_send` hook, if installed). */
  const sentRefs = async (draft: Rec, c: Capabilities): Promise<Rec[]> => {
    const prep = handlers.__ref_prepare;
    if (!c.slots?.length || !prep) return [];
    return (await prep({
      model_id: draft.model_id,
      references: draft.references ?? null,
    })) as Rec[];
  };
  /** FR-RGN-13/42: cost-affecting controls plus the ordered cost-affecting (slot, hash) list. */
  const estimateId = (draft: Rec, f: Rec, c: Capabilities, refs: Rec[]) => {
    const controls = (draft.controls ?? {}) as Rec;
    const costly = c.controls
      .filter((x) => x.cost_affecting)
      .map((x) => `${x.name}=${JSON.stringify(controls[x.name] ?? null)}`);
    const sent = refs.filter((r) => r.cost_affecting).map((r) => `${r.slot}=${r.content_hash}`);
    const tail = sent.length ? `:refs=${sent.join(',')}` : '';
    return `est:${String(draft.model_id)}:${f.sent_w}x${f.sent_h}:${costly.join(',')}${tail}`;
  };

  /** The prompt and seed are bound by role (R28); evaluate sees them under their names. A
   *  missing prompt only fails at submit (FR-RGN-09). */
  const checkDraft = async (draft: Rec, submit = false) => {
    const c = caps(draft.model_id);
    if (!c) return fail('unknown_model', 'unknown model');
    if (s.connectivity === 'offline')
      return fail('provider', 'offline', undefined, { provider: { code: 'offline' } });
    const region = draft.region as { inner: Rect; margin: number };
    const f = (await fit({
      inner: region.inner,
      margin: region.margin,
      model_id: draft.model_id,
    })) as Rec;
    if (!draft.mask_id) return fail('mask_invalid', 'The mask is empty.', 'empty');
    // pf-core pads the stored mask to the outer rect: one outside it is empty there (e2e only,
    // where edt-mock keeps the submitted masks).
    const stored = (w.__pfEdtMasks as Record<string, { rect: Rect }> | undefined)?.[
      String(draft.mask_id)
    ];
    const o = f.outer as Rect;
    if (
      stored &&
      (stored.rect.x >= o.x + o.w ||
        stored.rect.y >= o.y + o.h ||
        stored.rect.x + stored.rect.w <= o.x ||
        stored.rect.y + stored.rect.h <= o.y)
    )
      return fail('mask_invalid', 'paint the area to change inside the region first', 'empty');
    const values: Rec = { ...((draft.controls ?? {}) as Rec) };
    const promptCtl = c.controls.find((x) => x.role === 'prompt');
    const seedCtl = c.controls.find((x) => x.role === 'seed');
    if (promptCtl) values[promptCtl.name] = draft.final_prompt;
    if (seedCtl && draft.seed !== null) values[seedCtl.name] = draft.seed;
    const bad = evaluate(c, values).find((e) => e.error && (submit || e.name !== promptCtl?.name));
    if (bad) return fail('invalid_control', bad.error!.message, bad.name);
    return { c, f };
  };
  const needsOf = (cost: number | 'unknown') => {
    const needs: string[] = [];
    if (s.cap === null || s.threshold === null) needs.push('set_cap');
    if (cost === 'unknown') needs.push('unknown_cost');
    else if (s.threshold !== null && cost > s.threshold) needs.push('threshold');
    return needs;
  };
  const failOnce = (cmd: string) => {
    const f = s.failures[cmd];
    if (!f) return null;
    delete s.failures[cmd];
    return fail(f.code, f.message, f.field);
  };
  const slugOf = (prompt: string) => {
    const t = prompt
      .toLowerCase()
      .replace(/[^\p{L}\p{N}]+/gu, ' ')
      .trim();
    if (t.length <= 32) return t || 'edit';
    const cut = t.slice(0, 33);
    const sp = cut.lastIndexOf(' ');
    return (sp > 0 ? cut.slice(0, sp) : t.slice(0, 32)).trim() || 'edit';
  };
  const jobEvent = (j: Job, extra: Rec = {}) =>
    emit('rgn:job-state', {
      job_id: j.id,
      state: j.state,
      progress: j.progress,
      cost: { amount: j.cost, unit },
      error: j.error,
      ...extra,
    });
  // #56: resolving keeps every result (dismissed, still in results/); only discard trashes.
  const resolve = (j: Job) => {
    for (const c of s.cands) if (c.job_id === j.id && c.status === 'new') c.status = 'dismissed';
    if (j.state === 'fetched') j.state = 'closed';
    j.resolved = true;
    return 0;
  };

  Object.assign(handlers, {
    provider_list_models: () => ({
      models: seed.models.map((m) => ({ ...m, operations: ['edit_masked'] })),
      stale: !!seed.modelsStale,
    }),
    provider_get_capabilities: (input: Rec) =>
      caps(input.model_id) ??
      fail('provider', 'invalid_request(model_id)', 'model_id', {
        provider: { code: 'invalid_request' },
      }),
    rgn_region_fit: (input: Rec) => failOnce('rgn_region_fit') ?? fit(input),
    rgn_controls_evaluate: (input: Rec) => {
      const c = caps(input.model_id);
      if (!c) return fail('unknown_model', 'unknown model');
      return evaluateControls(c, (input.values ?? {}) as Rec);
    },
    rgn_job_estimate: async (input: Rec) => {
      const f1 = failOnce('rgn_job_estimate');
      if (f1) return f1;
      s.estimates++;
      const draft = input.draft as Rec;
      const ok = await checkDraft(draft);
      const refs = await sentRefs(draft, ok.c);
      const cost = costOf(ok.c, (draft.controls ?? {}) as Rec);
      return {
        estimate_id: estimateId(draft, ok.f, ok.c, refs),
        references: refs.length,
        reference_notices: [],
        cost: { amount: cost, unit },
        fit: ok.f,
        candidate_count: countOf(ok.c, (draft.controls ?? {}) as Rec),
        spend: s.spend,
        cap: s.cap,
        threshold: s.threshold,
        needs: needsOf(cost),
        over_cap: s.cap !== null && s.spend + (cost === 'unknown' ? 0 : cost) > s.cap,
      };
    },
    rgn_job_submit: async (input: Rec) => {
      const f1 = failOnce('rgn_job_submit');
      if (f1) return f1;
      if (snap.read_only) return fail('read_only', 'The project is read-only.');
      const draft = input.draft as Job['draft'];
      const ok = await checkDraft(draft, true);
      const refs = await sentRefs(draft, ok.c);
      if (estimateId(draft, ok.f, ok.c, refs) !== input.estimate_id)
        return fail('estimate_stale', 'The estimate is out of date.');
      if (active()) return fail('job_active', 'A job is already running in this project.');
      const cost = costOf(ok.c, draft.controls);
      const amount = cost === 'unknown' ? 0 : cost;
      if (s.cap !== null && s.spend + amount > s.cap)
        return fail('over_spend_cap', 'This job would go over the spend cap.', undefined, {
          spend: s.spend,
          estimate: cost,
          cap: s.cap,
        });
      const given = (input.confirmations ?? []) as string[];
      const missing = needsOf(cost).filter((n) => !given.includes(n));
      if (missing.length) return fail('confirmation_missing', missing.join(','));
      const prev = newest();
      if (prev && !prev.resolved) resolve(prev);
      const f = ok.f as { outer: Rect };
      const j: Job = {
        id: id('01JJOB'),
        seq: s.jobs.length,
        state: 'submitted',
        draft: JSON.parse(JSON.stringify(draft)),
        outer: f.outer,
        cost,
        n: null,
        dir: null,
        slug: null,
        resolved: false,
        error: null,
        progress: 0,
        refs: refs.map((r) => {
          const row = { ...r };
          delete row.cost_affecting;
          return row;
        }),
      };
      s.jobs.push(j);
      s.spend += amount;
      emit('rgn:spend', { spend: s.spend, cap: s.cap });
      jobEvent(j);
      sync();
      return { job_id: j.id };
    },
    rgn_job_cancel: (input: Rec) => {
      const j = s.jobs.find((x) => x.id === input.job_id);
      if (!j) return fail('not_found', 'no such job');
      if (!['submitted', 'queued', 'running'].includes(j.state) || s.cancel === 'not_cancellable')
        return { result: 'not_cancellable' };
      j.state = 'canceled';
      jobEvent(j);
      return { result: 'canceled' };
    },
    rgn_job_retry_draft: async (input: Rec) => {
      const j = s.jobs.find((x) => x.id === input.job_id);
      if (!j) return fail('not_found', 'no such job');
      // FR-RGN-44 / FR-REF-26: the job's references back as the selection.
      const retry = handlers.__ref_retry;
      const references = retry
        ? await retry({
            rows: j.refs,
            label: j.n === null ? 'the job' : String(j.n).padStart(3, '0'),
            has_slots: !!caps(j.draft.model_id)?.slots?.length,
          })
        : null;
      return { ...JSON.parse(JSON.stringify(j.draft)), seed: j.draft.seed ?? null, references };
    },
    rgn_job_dismiss: (input: Rec) => {
      const j = s.jobs.find((x) => x.id === input.job_id);
      if (!j) return fail('not_found', 'no such job');
      if (active() === j) return fail('job_active', 'The job is still running.');
      const trashed = resolve(j);
      jobEvent(j);
      committed();
      return { trashed };
    },
    rgn_candidate_preview: (input: Rec) => {
      const j = s.jobs.find((x) => x.id === input.job_id);
      const c = s.cands.find((x) => x.id === input.candidate_id);
      if (!j || !c || c.status === 'invalid')
        return fail('invalid_candidate', 'The candidate can’t be used.');
      const key = `pv-${c.id}-${(input.blend as Rec).feather_px}`;
      return { key, url: `http://pf-tiles.localhost/i/${key}.png`, rect: j.outer };
    },
    rgn_candidate_accept: (input: Rec) => {
      if (snap.read_only) return fail('read_only', 'The project is read-only.');
      const j = s.jobs.find((x) => x.id === input.job_id);
      const c = s.cands.find((x) => x.id === input.candidate_id);
      if (!j || !c || c.status === 'invalid')
        return fail('invalid_candidate', 'The candidate can’t be used.');
      if (j !== s.jobs.filter((x) => x.n !== null).at(-1) || (j.state === 'closed' && inStack(j)))
        return fail('repick_unavailable', 'Only the newest iteration can be accepted.');
      const layer = {
        ...original,
        id: id('01JLAYERP'),
        kind: 'patch',
        file: `.pforge/layers/${c.id}.png`,
        job_id: j.id,
        candidate_id: c.id,
        rect: j.outer,
        blend: input.blend,
      };
      s.layers = [...s.layers.filter((l) => l.job_id !== j.id), layer];
      c.status = 'accepted';
      c.trash_id = null;
      resolve(j);
      j.state = 'closed';
      const rev = {
        id: id('01JREV'),
        seq: s.revs.length + 1,
        layers: s.layers.map((l) => ({ ...l })),
        reason: 'accept',
        label: j.draft.final_prompt.slice(0, 40),
      };
      s.revs.push(rev);
      jobEvent(j);
      committed();
      return { layer_id: layer.id, revision_id: rev.id };
    },
    rgn_candidate_star: (input: Rec) => {
      const c = s.cands.find((x) => x.id === input.candidate_id && x.job_id === input.job_id);
      if (!c) return fail('not_found', 'no such candidate');
      c.starred = !!input.starred;
      if (c.starred) c.trash_id = null;
      committed();
      return {};
    },
    rgn_candidate_discard: (input: Rec) => {
      const c = s.cands.find((x) => x.id === input.candidate_id && x.job_id === input.job_id);
      if (!c) return fail('not_found', 'no such candidate');
      if (c.status === 'accepted')
        return fail('invalid_input', 'An accepted candidate can’t be discarded.');
      c.status = 'discarded';
      c.starred = false;
      c.trash_id = id('T');
      committed();
      return {};
    },
    rgn_job_discard_rest: (input: Rec) => {
      const j = s.jobs.find((x) => x.id === input.job_id);
      if (!j) return fail('not_found', 'no such job');
      if (active() === j) return fail('job_active', 'The job is still running.');
      const rest = s.cands.filter(
        (c) =>
          c.job_id === j.id &&
          c.status !== 'accepted' &&
          !c.starred &&
          !(c.status === 'discarded' && c.trash_id),
      );
      if (!rest.length) return { trashed: 0 };
      const tid = id('T');
      let trashed = 0;
      for (const c of rest) {
        c.status = 'discarded';
        if (!c.trash_id) {
          c.trash_id = tid;
          trashed++;
        }
      }
      committed();
      return { trashed };
    },
    rgn_patch_retune: (input: Rec) => {
      const l = s.layers.find((x) => x.id === input.layer_id);
      if (!l) return fail('source_missing', 'no such layer');
      const nl = { ...l, id: id('01JLAYERP'), blend: input.blend, retune_of: l.id };
      s.layers = s.layers.map((x) => (x === l ? nl : x));
      const rev = {
        id: id('01JREV'),
        seq: s.revs.length + 1,
        layers: [...s.layers],
        reason: 'retune',
      };
      s.revs.push(rev);
      committed();
      return { layer_id: nl.id, revision_id: rev.id };
    },
    rgn_iterations_list: () => {
      const rows = s.jobs
        .filter((j) => j.n !== null || (j === newest() && !j.resolved))
        .map((j) => {
          const mine = s.cands.filter((c) => c.job_id === j.id);
          return {
            job_id: j.id,
            n: j.n,
            dir: j.dir,
            slug: j.slug,
            state: j.state,
            kept: mine.filter((c) => c.trash_id === null && c.status !== 'invalid').length,
            returned: mine.length,
            cost: { amount: j.cost, unit },
            accepted_in_stack: inStack(j),
            references: j.refs.length,
            reference_files: j.refs.map((r) => ({
              file: String(r.path).split('/').pop(),
              pack_name: r.pack_name ?? null,
              role: r.role ?? null,
            })),
          };
        });
      return rows.reverse();
    },
    project_put_draft: (input: Rec) => {
      const f = failOnce('project_put_draft');
      if (f) return f;
      if (snap.read_only) return fail('read_only', 'The project is read-only.');
      snap.draft = { ...(snap.draft as Rec), ...(input.draft as Rec) };
      return { seq: s.nextId++ };
    },
    rgn_spend_get: () => ({ spend: s.spend, cap: s.cap, threshold: s.threshold }),
    rgn_blend_instruction_get: () => blendOut(),
    rgn_blend_instruction_set: (input: Rec) => {
      const t = input.text;
      if (t !== null && typeof t !== 'string') return fail('invalid_input', 'text');
      if (typeof t === 'string' && t.length > 2000)
        return fail('invalid_input', 'the instruction is at most 2000 characters', 'text');
      s.blendText = t === s.blendDefault ? null : t;
      const out = blendOut();
      emit('rgn:blend-instruction', out);
      return out;
    },
    // A missing field keeps its value; null clears it (pf-core `rgn_spend_set`).
    rgn_spend_set: (input: Rec) => {
      const next = { cap: s.cap, threshold: s.threshold };
      for (const k of ['cap', 'threshold'] as const) {
        if (!(k in input)) continue;
        const v = input[k];
        if (v === null) next[k] = null;
        else if (typeof v === 'number' && Number.isFinite(v) && v >= 0) next[k] = v;
        else return fail('invalid_value', 'must be a number ≥ 0', k);
      }
      s.cap = next.cap;
      s.threshold = next.threshold;
      emit('rgn:spend', { spend: s.spend, cap: s.cap });
      return { spend: s.spend, cap: s.cap, threshold: s.threshold };
    },
    project_restore_revision: (input: Rec) => {
      const r = s.revs.find((x) => x.id === input.rev_id);
      // r0's draft is null (a fresh project); the mock doesn't keep later revisions' drafts.
      if (input.rev_id === 'r0') {
        s.layers = [original];
        snap.draft = null;
      } else if (!r) return fail('unknown_id', 'no such revision');
      else s.layers = r.layers.map((l) => ({ ...l }));
      const rev = {
        id: id('01JREV'),
        seq: s.revs.length + 1,
        layers: [...s.layers],
        reason: 'restore',
      };
      s.revs.push(rev);
      committed();
      return {
        id: rev.id,
        seq: rev.seq,
        created_at: '2026-11-07T11:00:00.000Z',
        reason: 'restore',
      };
    },
  });
  function inStack(j: Job): boolean {
    return s.layers.some((l) => l.job_id === j.id);
  }

  /** Test controls: drive the active job through Appendix B. */
  w.__pfRgnMock = {
    state: () => s,
    setConnectivity(c: string) {
      s.connectivity = c;
    },
    setCancel(c: 'canceled' | 'not_cancellable') {
      s.cancel = c;
    },
    failNext(cmd: string, code: string, message = code, field?: string) {
      s.failures[cmd] = { code, message, ...(field ? { field } : {}) };
    },
    /** submitted → running(progress) */
    run(progress = 0.5, extra: Rec = {}) {
      const j = active();
      if (!j) return;
      j.state = 'running';
      j.progress = progress;
      jobEvent(j, extra);
    },
    /** → succeeded → fetched with `n` candidates (index `invalid` ones won't decode, index
     *  `odd` ones came back in another aspect, FR-RGN-24). */
    finish(count?: number, invalid: number[] = [], odd: number[] = []) {
      const j = active();
      if (!j) return;
      j.state = 'succeeded';
      j.progress = 1;
      jobEvent(j);
      const c = caps(j.draft.model_id)!;
      const k = count ?? countOf(c, j.draft.controls);
      j.n = Math.max(0, ...s.jobs.map((x) => x.n ?? 0)) + 1;
      j.slug = slugOf(j.draft.final_prompt);
      j.dir = `results/${String(j.n).padStart(3, '0')} ${j.slug}`;
      for (let i = 0; i < k; i++) {
        const cand: Cand = {
          id: `${j.id}C${i + 1}`,
          job_id: j.id,
          index: i,
          file: `${j.dir}/${String(i + 1).padStart(2, '0')}.png`,
          status: invalid.includes(i) ? 'invalid' : 'new',
          starred: false,
          trash_id: null,
          width: j.outer.w,
          height: j.outer.h,
          ...(odd.includes(i) ? { aspect_warning: true } : {}),
        };
        s.cands.push(cand);
      }
      j.state = 'fetched';
      committed();
      for (const cand of s.cands.filter((x) => x.job_id === j.id))
        emit('rgn:candidate-ready', {
          job_id: j.id,
          candidate_id: cand.id,
          status: cand.status === 'invalid' ? 'invalid' : 'ready',
        });
      jobEvent(j);
    },
    /** → failed with a normalised error. */
    fail(code = 'provider_error', hint = 'Try again later.') {
      const j = active();
      if (!j) return;
      j.state = 'failed';
      j.error = { code, hint };
      jobEvent(j);
    },
  };
  sync();
}
