// Mock of the `gen_*` commands (new-asset-generation §9) plus PRV's `generate` model list and
// capabilities, for Vitest and Playwright. It is the UI's contract with GEN core (slices 1, 3):
// size fit (FR-GEN-08/09), controls evaluate, estimate and submit through **RGN's job runner**
// (the job joins the RGN mock's jobs, so the tracker, `__pfRgnMock.finish()` and the one-active-
// job rule are shared, FR-GEN-22/23), and the retry draft (FR-GEN-32).
//
// Install it **after** `installRgnMock` (it reads `window.__pfRgnMock` and wraps the PRV model
// handlers). Like the RGN mock it is **self-contained** (Playwright serialises it with
// `toString()`): nothing outside the function body is referenced. Never imported by app code.
import type { Capabilities } from '../types';

export interface GenMockSeed {
  /** Models `provider_list_models{operation: generate}` lists. */
  models: { id: string; name: string }[];
  /** Their `generate` capabilities (`provider_get_capabilities{operation: generate}`). */
  capabilities: Record<string, Capabilities>;
  /** Cost per image; `'unknown'` → the estimate is unknown. */
  costPerImage?: number | 'unknown';
  modelsStale?: boolean;
}

export function installGenMock(seed: GenMockSeed): void {
  type Rec = Record<string, unknown>;
  type Size =
    | { mode: 'grid'; cols: number; rows: number; pps: number }
    | { mode: 'ratio'; aspect: string; long_side: number };

  const w = window as unknown as Rec;
  const handlers = (w.__pfMockHandlers ??= {}) as Record<string, (input: Rec) => unknown>;
  const snap = (w.__pfMockSnapshot ??= {}) as Rec;
  const rgn = () =>
    (w.__pfRgnMock as { state(): Rec } | undefined)?.state() as
      | (Rec & { jobs: Rec[]; cands: Rec[]; revs: Rec[]; spend: number; cap: number | null })
      | undefined;
  const fail = (code: string, message: string, field?: string, detail?: unknown) =>
    Promise.reject({ code, message, ...(field ? { field } : {}), ...(detail ? { detail } : {}) });
  const emit = (event: string, payload: unknown) =>
    (w.__pfMock as { emit?: (e: string, p: unknown) => void } | undefined)?.emit?.(event, payload);
  const caps = (id: unknown): Capabilities | undefined => seed.capabilities[String(id)];
  const g = {
    /** Sketch file → its content version; a test bumps it to flag `changed since NNN`. */
    sketchVersion: {} as Record<string, number>,
    /** Generate job id → its draft and the sketch version it sent. */
    jobs: {} as Record<
      string,
      { draft: Rec; sketch: { file: string; v: number } | null; helpers: Rec[] }
    >,
  };

  // ── FR-GEN-08/09/10: size fit ──────────────────────────────────────────────────────────────
  const isRatio = (v: string) => /^\d+(\.\d+)?:\d+(\.\d+)?$/.test(v);
  const ratioOf = (v: string) => {
    const [a, b] = v.split(':').map(Number);
    return a! / b!;
  };
  const sizeFit = (c: Capabilities, size: Size) => {
    const o = c.output_size ?? {
      mode: 'wh' as const,
      min: 16,
      max: 4096,
      step: 16,
      min_pixels: null,
    };
    const notices: { code: string; message: string }[] = [];
    if (o.mode === 'aspect') {
      const target =
        size.mode === 'grid'
          ? size.cols / size.rows
          : isRatio(size.aspect)
            ? ratioOf(size.aspect)
            : 1;
      const ratios = o.allowed.filter(isRatio);
      let best = ratios[0] ?? '1:1';
      for (const r of ratios)
        if (
          Math.abs(Math.log(ratioOf(r)) - Math.log(target)) <
          Math.abs(Math.log(ratioOf(best)) - Math.log(target))
        )
          best = r;
      if (size.mode === 'grid' && Math.abs(ratioOf(best) / target - 1) > 0.01)
        notices.push({ code: 'cells_not_square', message: 'Cells won’t be square.' });
      return { mode: 'aspect', aspect: best, pps: size.mode === 'grid' ? size.pps : null, notices };
    }
    const fit = (v: number) => Math.max(o.min, Math.floor(v / o.step) * o.step);
    // Under `min_pixels`: both sides scaled up from the ratio, rounded up to the step, within max.
    const minArea = (w: number, h: number, r: number): [number, number, boolean] => {
      const m = o.min_pixels;
      if (!m || w * h >= m) return [w, h, false];
      const top = Math.max(o.min, Math.floor(o.max / o.step) * o.step);
      const up = (v: number) => Math.min(top, Math.ceil(Math.ceil(v) / o.step) * o.step);
      const ih = Math.sqrt(m / r);
      let [sw, sh] = [Math.max(w, up(ih * r)), Math.max(h, up(ih))];
      while (sw * sh < m && (sw < top || sh < top)) {
        if (sh >= top || (sw < top && (sw + o.step) * sh <= sw * (sh + o.step)))
          sw = Math.min(top, sw + o.step);
        else sh = Math.min(top, sh + o.step);
      }
      notices.push({
        code: 'model_minimum',
        message: `Model minimum: sent ${sw} × ${sh} (at least ${String(m).replace(/\B(?=(\d{3})+(?!\d))/g, ' ')} px in all).`,
      });
      return [sw, sh, true];
    };
    if (size.mode === 'grid') {
      let pps = size.pps;
      const longest = Math.max(size.cols, size.rows);
      if (longest * pps > o.max) {
        pps = Math.floor(o.max / longest);
        notices.push({
          code: 'model_limit',
          message: `Model limit: ${pps} px per square (at most ${o.max} px a side).`,
        });
      }
      const [w, h, scaled] = minArea(
        fit(size.cols * pps),
        fit(size.rows * pps),
        size.cols / size.rows,
      );
      if (scaled) pps = Math.max(1, Math.min(Math.floor(w / size.cols), Math.floor(h / size.rows)));
      return { mode: 'wh', w, h, pps, notices };
    }
    const r = isRatio(size.aspect) ? ratioOf(size.aspect) : 1;
    const long = Math.min(o.max, size.long_side);
    const [ww, hh] = r >= 1 ? [long, long / r] : [long * r, long];
    const [w, h] = minArea(fit(ww), fit(hh), r);
    return { mode: 'wh', w, h, pps: null, notices };
  };
  const sizeOf = (c: Capabilities, size: Size) => {
    const f = sizeFit(c, size) as Rec & { w?: number; h?: number; aspect?: string };
    if (f.mode === 'wh') return { f, w: f.w!, h: f.h! };
    const r = ratioOf(String(f.aspect));
    return { f, w: r >= 1 ? 1024 : Math.round(1024 * r), h: r >= 1 ? Math.round(1024 / r) : 1024 };
  };

  // ── FR-GEN-06: evaluate (allowed values, required) as PRV's evaluate, simplified ───────────
  const evaluate = (c: Capabilities, given: Rec) => {
    const values: Rec = {};
    for (const [k, v] of Object.entries(given)) if (v !== null && v !== undefined) values[k] = v;
    const unknown = Object.keys(values).filter((k) => !c.controls.some((x) => x.name === k));
    const required_missing: string[] = [];
    const controls = c.controls
      .filter((x) => !['source', 'mask', 'reference'].includes(x.role))
      .filter((x) => x.type !== 'image' && x.type !== 'mask')
      .map((x) => {
        let value: unknown = values[x.name] ?? x.default ?? null;
        let reset = false;
        const bad =
          value !== null &&
          ((x.allowed.length && !x.allowed.includes(value)) ||
            (typeof value === 'number' &&
              ((x.min !== null && value < x.min) || (x.max !== null && value > x.max))));
        if (bad) {
          value = x.default ?? x.allowed[0] ?? null;
          reset = true;
        }
        const required = x.required || (x.role === 'prompt' && c.prompt_required);
        const empty = value === null || value === '';
        if (required && empty) required_missing.push(x.name);
        return {
          name: x.name,
          type: x.type,
          role: x.role,
          min: x.min,
          max: x.max,
          step: x.step,
          default: x.default,
          cost_affecting: x.cost_affecting,
          visible: true,
          allowed: x.allowed,
          required,
          value,
          reset,
          ...(required && empty
            ? { error: { control: x.name, code: 'required', message: `${x.name} is required` } }
            : {}),
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

  const countOf = (c: Capabilities, controls: Rec) => {
    const ctl = c.controls.find((x) => x.role === 'candidate_count');
    return { ctl, n: Number(ctl ? (controls[ctl.name] ?? ctl.default ?? 1) : 1) };
  };
  const costOf = (c: Capabilities, controls: Rec): number | 'unknown' =>
    seed.costPerImage === 'unknown' ? 'unknown' : countOf(c, controls).n * (seed.costPerImage ?? 1);

  /** The project's `structure/` listing (PRJ's mock, when installed). */
  const listStructure = () =>
    Promise.resolve(handlers.project_folder_list?.({ folder: 'structure' })).catch(() => undefined);

  /** FR-GEN-13/15: REF's prepare (the REF mock's hook) plus the sketch row, first in its slot. */
  const sentRefs = async (draft: Rec, c: Capabilities): Promise<Rec[]> => {
    const file = (draft.structure as { file?: string } | null)?.file ?? null;
    const rows: Rec[] = [];
    if (file && c.structure) {
      const folder = (await listStructure()) as { name: string }[] | undefined;
      if (folder && !folder.some((f) => f.name === file))
        return fail(
          'reference_unavailable',
          `The sketch ${file} can’t be sent: it can’t be found`,
          undefined,
          {
            key: `structure:${file}`,
            reason: 'missing',
          },
        );
      rows.push({
        source: 'structure',
        pack_id: null,
        pack_name: null,
        path: `structure/${file}`,
        content_hash: `${file}#${g.sketchVersion[file] ?? 0}`,
        slot: c.structure.slot,
        role: null,
        cost_affecting: !!c.slots.find((s) => s.name === c.structure!.slot)?.cost_affecting,
      });
    }
    const prep = handlers.__ref_prepare;
    if (c.slots.length && prep)
      rows.push(
        ...((await prep({
          model_id: draft.model_id,
          references: draft.references ?? null,
          operation: 'generate',
          structure: file,
        })) as Rec[]),
      );
    return rows;
  };

  const estimateId = (draft: Rec, c: Capabilities, size: Rec, refs: Rec[]) => {
    const controls = (draft.controls ?? {}) as Rec;
    const costly = c.controls
      .filter((x) => x.cost_affecting)
      .map((x) => `${x.name}=${JSON.stringify(controls[x.name] ?? null)}`);
    const sent = refs.filter((r) => r.cost_affecting).map((r) => `${r.slot}=${r.content_hash}`);
    return `gen:${String(draft.model_id)}:${size.w}x${size.h}:${costly.join(',')}:${sent.join(',')}`;
  };

  const check = async (draft: Rec, submit = false) => {
    const c = caps(draft.model_id);
    if (!c) return fail('unknown_model', 'unknown model');
    if (rgn()?.connectivity === 'offline')
      return fail('provider', 'offline', undefined, { provider: { code: 'offline' } });
    if (submit && c.prompt_required && !String(draft.final_prompt ?? '').trim())
      return fail('invalid_control', 'prompt is required', 'prompt');
    const { f, w: sw, h: sh } = sizeOf(c, draft.size as Size);
    const refs = await sentRefs(draft, c);
    return { c, f, sw, sh, refs };
  };
  const needsOf = (cost: number | 'unknown') => {
    const s = rgn()!;
    const needs: string[] = [];
    if (s.cap === null || s.threshold === null) needs.push('set_cap');
    if (cost === 'unknown') needs.push('unknown_cost');
    else if (s.threshold !== null && cost > Number(s.threshold)) needs.push('threshold');
    return needs;
  };
  const outputOf = (f: Rec, sw: number, sh: number, size: Size) => ({
    mode: f.mode,
    ...(f.mode === 'wh' ? { w: sw, h: sh } : { aspect: f.aspect }),
    ...(size.mode === 'grid' ? { grid: { cols: size.cols, rows: size.rows, pps: f.pps } } : {}),
  });

  const prevList = handlers.provider_list_models;
  const prevCaps = handlers.provider_get_capabilities;
  Object.assign(handlers, {
    provider_list_models: (input: Rec) =>
      input.operation === 'generate'
        ? {
            models: seed.models.map((m) => ({ ...m, operations: ['generate'] })),
            stale: !!seed.modelsStale,
          }
        : prevList?.(input),
    provider_get_capabilities: (input: Rec) =>
      input.operation === 'generate'
        ? (caps(input.model_id) ??
          fail('provider', 'invalid_request(model_id)', 'model_id', {
            provider: { code: 'invalid_request' },
          }))
        : prevCaps?.(input),
    gen_size_fit: (input: Rec) => {
      const c = caps(input.model_id);
      if (!c) return fail('unknown_model', 'unknown model');
      return sizeFit(c, input.size as Size);
    },
    gen_controls_evaluate: (input: Rec) => {
      const c = caps(input.model_id);
      if (!c) return fail('unknown_model', 'unknown model');
      return evaluate(c, (input.values ?? {}) as Rec);
    },
    gen_job_estimate: async (input: Rec) => {
      const draft = input.draft as Rec;
      const ok = await check(draft);
      const s = rgn()!;
      const cost = costOf(ok.c, (draft.controls ?? {}) as Rec);
      return {
        estimate_id: estimateId(draft, ok.c, { w: ok.sw, h: ok.sh }, ok.refs),
        cost: { amount: cost, unit: 'CU' },
        candidate_count: countOf(ok.c, (draft.controls ?? {}) as Rec).n,
        spend: s.spend,
        cap: s.cap,
        threshold: s.threshold,
        needs: needsOf(cost),
        over_cap: s.cap !== null && s.spend + (cost === 'unknown' ? 0 : cost) > s.cap,
        references: ok.refs.filter((r) => r.source !== 'structure').length,
        reference_notices: [],
        output: outputOf(ok.f, ok.sw, ok.sh, draft.size as Size),
      };
    },
    gen_job_submit: async (input: Rec) => {
      if (snap.read_only) return fail('read_only', 'The project is read-only.');
      const draft = input.draft as Rec;
      const ok = await check(draft, true);
      const s = rgn()!;
      if (estimateId(draft, ok.c, { w: ok.sw, h: ok.sh }, ok.refs) !== input.estimate_id)
        return fail('estimate_stale', 'The estimate is out of date.');
      const terminal = (st: unknown) =>
        ['failed', 'canceled', 'lost', 'closed'].includes(String(st));
      if (s.jobs.some((j) => !terminal(j.state) && j.state !== 'fetched'))
        return fail('job_active', 'A job is already running in this project.');
      const cost = costOf(ok.c, (draft.controls ?? {}) as Rec);
      const amount = cost === 'unknown' ? 0 : cost;
      if (s.cap !== null && s.spend + amount > s.cap)
        return fail('over_spend_cap', 'This job would go over the spend cap.');
      const given = (input.confirmations ?? []) as string[];
      const missing = needsOf(cost).filter((n) => !given.includes(n));
      if (missing.length) return fail('confirmation_missing', missing.join(','));
      // The next submit resolves the previous iteration (#56: its candidates are kept).
      const prev = s.jobs.at(-1);
      if (prev && !prev.resolved) {
        for (const c of s.cands)
          if (c.job_id === prev.id && c.status === 'new') c.status = 'dismissed';
        if (prev.state === 'fetched') prev.state = 'closed';
        prev.resolved = true;
      }
      const count = countOf(ok.c, (draft.controls ?? {}) as Rec);
      const id = `01JGENJOB${String(s.jobs.length + 1).padStart(4, '0')}`;
      const sketch = (draft.structure as { file?: string } | null)?.file ?? null;
      const job: Rec = {
        id,
        seq: s.jobs.length,
        state: 'submitted',
        operation: 'generate',
        output: outputOf(ok.f, ok.sw, ok.sh, draft.size as Size),
        draft: {
          ...JSON.parse(JSON.stringify(draft)),
          final_prompt: String(draft.typed_prompt ?? ''),
          region: { inner: { x: 0, y: 0, w: ok.sw, h: ok.sh }, margin: 0 },
        },
        outer: { x: 0, y: 0, w: ok.sw, h: ok.sh },
        cost,
        n: null,
        dir: null,
        slug: null,
        resolved: false,
        error: null,
        progress: 0,
        refs: ok.refs
          .filter((r) => r.source !== 'structure')
          .map((r) => {
            const row = { ...r };
            delete row.cost_affecting;
            return row;
          }),
        sent: {
          controls: {},
          count: count.ctl ? { name: count.ctl.name, n: count.n } : { name: '', n: 1 },
        },
      };
      s.jobs.push(job);
      // FR-PRM-09: the helpers sent (the PRM mock's compose hook); never the blend instruction.
      const kind = String((handlers.project_get?.({}) as Rec | undefined)?.kind ?? 'map');
      const helpers = handlers.__prm_sent
        ? ((await handlers.__prm_sent({
            kind,
            helper_ids: (draft.prompt_helpers as string[] | null | undefined) ?? null,
            blend_instruction: null,
          })) as Rec[])
        : [];
      g.jobs[id] = {
        draft: JSON.parse(JSON.stringify(draft)),
        sketch: sketch ? { file: sketch, v: g.sketchVersion[sketch] ?? 0 } : null,
        helpers,
      };
      s.spend = Number(s.spend) + amount;
      emit('rgn:spend', { spend: s.spend, cap: s.cap });
      emit('rgn:job-state', {
        job_id: id,
        state: 'submitted',
        progress: 0,
        cost: { amount: cost, unit: 'CU' },
        error: null,
      });
      return { job_id: id };
    },
    gen_job_retry_draft: async (input: Rec) => {
      const rec = g.jobs[String(input.job_id)];
      if (!rec) {
        const exists = rgn()?.jobs.some((j) => j.id === input.job_id);
        return exists
          ? fail('wrong_operation', 'Not a generate job.')
          : fail('not_found', 'no such job');
      }
      const job = rgn()!.jobs.find((j) => j.id === input.job_id)!;
      const n = job.n === null ? 'the job' : String(job.n).padStart(3, '0');
      const sk = rec.sketch;
      let structure: Rec | null = null;
      if (sk) {
        const folder = (await listStructure()) as { name: string }[] | undefined;
        const present = !folder || folder.some((f) => f.name === sk.file);
        structure = !present
          ? { file: sk.file, missing: true }
          : (g.sketchVersion[sk.file] ?? 0) !== sk.v
            ? { file: sk.file, changed_since: n }
            : { file: sk.file };
      }
      const retry = handlers.__ref_retry;
      const rows = (job.refs ?? []) as Rec[];
      const references = retry
        ? await retry({ rows, label: n, has_slots: !!caps(rec.draft.model_id)?.slots.length })
        : (rec.draft.references ?? null);
      // FR-PRM-10: the job's helpers ticked again; changed / missing badged.
      const kind = String((handlers.project_get?.({}) as Rec | undefined)?.kind ?? 'map');
      const prm = handlers.__prm_retry
        ? ((await handlers.__prm_retry({ kind, recorded: rec.helpers, since: n })) as {
            ticks: string[];
            prompt_retry?: Rec;
          })
        : { ticks: (rec.draft.prompt_helpers as string[] | null | undefined) ?? [] };
      return {
        ...JSON.parse(JSON.stringify(rec.draft)),
        seed: rec.draft.seed ?? null,
        structure,
        references,
        prompt_helpers: prm.ticks,
        ...(prm.prompt_retry ? { prompt_retry: prm.prompt_retry } : {}),
      };
    },
  });

  // ── FR-GEN-26/27/29/30: preview, pick, and undo back to no image ───────────────────────────
  type Layer = Rec & { kind?: string; width?: number; height?: number; file?: string };
  /** The merged snapshot as `project_get` serves it (PRJ's, or the snapshot overrides). */
  const view = (): Rec => {
    const p = handlers.project_get?.({});
    return { ...(p && !(p instanceof Promise) ? (p as Rec) : {}), ...snap };
  };
  /** The project had no image at r0 (Generate new): undo to r0 returns to "No image yet". */
  let r0NoImage: boolean | null = null;
  /** Snapshot image fields from the bottom layer, as pf-core's snapshot has them. */
  const applyImage = (layers: Layer[]) => {
    const l = layers[0];
    if (l?.kind === 'base') {
      Object.assign(snap, {
        image_path: l.file,
        original_image_id: l.id,
        has_image: true,
        width: l.width,
        height: l.height,
        format: 'png',
      });
    } else if (l) {
      for (const k of ['image_path', 'original_image_id', 'has_image', 'width', 'height', 'format'])
        delete snap[k];
    } else {
      Object.assign(snap, {
        image_path: null,
        original_image_id: null,
        has_image: false,
        width: 0,
        height: 0,
        format: null,
      });
    }
  };
  const rgnMock = () => w.__pfRgnMock as { state(): Rec; commit(): void };
  type GenCand = Rec & {
    id: string;
    job_id: string;
    status: string;
    width: number;
    height: number;
  };
  const lookup = (input: Rec) => {
    const s = rgn()!;
    const j = s.jobs.find((x) => x.id === input.job_id);
    if (!j) return { err: fail('not_found', 'no such job', 'job_id') };
    if (j.operation !== 'generate')
      return { err: fail('wrong_operation', 'Accept it from Region edit.', 'job_id') };
    const c = (s.cands as GenCand[]).find((x) => x.id === input.candidate_id);
    if (!c) return { err: fail('not_found', 'no such candidate', 'candidate_id') };
    if (c.job_id !== j.id || c.status === 'invalid' || c.status === 'discarded')
      return { err: fail('invalid_candidate', 'The candidate can’t be used.', 'candidate_id') };
    return { s, j, c };
  };
  const prevRestore = handlers.project_restore_revision;
  Object.assign(handlers, {
    gen_candidate_preview: (input: Rec) => {
      const r = lookup(input);
      if (r.err) return r.err;
      const v = view();
      const key = `gp-${r.c.id}`;
      return {
        key,
        url: `http://pf-tiles.localhost/i/${key}.png`,
        rect: { x: 0, y: 0, w: r.c.width, h: r.c.height },
        same_size: !!v.image_path && v.width === r.c.width && v.height === r.c.height,
      };
    },
    gen_candidate_pick: (input: Rec) => {
      if (snap.read_only) return fail('read_only', 'The project is read-only.');
      // R97: never while a job runs (a region edit job would come back for another base image).
      const running = rgn()!.jobs.find(
        (x) => !['failed', 'canceled', 'lost', 'closed', 'fetched'].includes(String(x.state)),
      );
      if (running)
        return fail(
          'job_active',
          'a job is running in this project; pick when it has finished',
          undefined,
          { job_id: running.id },
        );
      const r = lookup(input);
      if (r.err) return r.err;
      const { s, j, c } = r;
      r0NoImage ??= !view().image_path;
      const layers = s.layers as Layer[];
      const dropped = layers.filter((l) => l.kind === 'patch').length;
      const n = String((c.index as number) + 1).padStart(2, '0');
      const layer: Layer = {
        id: `01JLAYERB${String(s.revs.length + 1).padStart(4, '0')}${c.id.slice(-4)}`,
        kind: 'base',
        file: `working/${String(j.dir).replace(/^results\//, '')} ${n}.png`,
        content_hash: `sha256:${c.id}`,
        width: c.width,
        height: c.height,
        job_id: j.id,
        candidate_id: c.id,
        cx: c.width / 2,
        cy: c.height / 2,
        scale: 1,
        rotation: 0,
        opacity: 1,
        visible: true,
      };
      s.layers = [layer];
      // R84: only the current base candidate stays accepted (the previous one is kept, dismissed).
      const genJobs = new Set(s.jobs.filter((x) => x.operation === 'generate').map((x) => x.id));
      for (const o of s.cands as GenCand[])
        if (o !== c && o.status === 'accepted' && genJobs.has(o.job_id)) o.status = 'dismissed';
      c.status = 'accepted';
      c.trash_id = null;
      if (j.state === 'fetched') {
        for (const o of s.cands as GenCand[])
          if (o.job_id === j.id && o.status === 'new') o.status = 'dismissed';
        j.state = 'closed';
        j.resolved = true;
      }
      const typed = String((j.draft as Rec).typed_prompt ?? (j.draft as Rec).final_prompt ?? '');
      const rev = {
        id: `01JREVG${String(s.revs.length + 1).padStart(4, '0')}`,
        seq: s.revs.length + 1,
        layers: [{ ...layer }],
        reason: 'accept',
        label: `New: ${typed.trim().slice(0, 40)}`,
      };
      s.revs.push(rev);
      applyImage(s.layers as Layer[]);
      rgnMock().commit();
      emit('rgn:job-state', { job_id: j.id, state: j.state, progress: 1, error: null });
      return {
        layer_id: layer.id,
        revision_id: rev.id,
        file: layer.file,
        patches_dropped: dropped,
      };
    },
    project_restore_revision: async (input: Rec) => {
      const out = await prevRestore?.(input);
      const s = rgn();
      if (s) {
        if (input.rev_id === 'r0' && r0NoImage) s.layers = [];
        // R97 (R84 across undo/redo): among generate candidates only the restored base image's
        // is `accepted`; a trashed one comes back with it (the mock's files never go away).
        const layers = s.layers as Layer[];
        const base =
          layers[0]?.kind === 'base'
            ? ((layers[0].candidate_id as string | undefined) ?? null)
            : null;
        const genJobs = new Set(s.jobs.filter((x) => x.operation === 'generate').map((x) => x.id));
        for (const c of s.cands as GenCand[]) {
          if (!genJobs.has(c.job_id)) continue;
          if (c.id === base) {
            if (c.status !== 'invalid') {
              c.status = 'accepted';
              c.trash_id = null;
            }
          } else if (c.status === 'accepted') c.status = 'dismissed';
        }
        applyImage(s.layers as Layer[]);
        rgnMock().commit();
      }
      return out;
    },
  });

  /** Test controls. */
  w.__pfGenMock = {
    /** The sketch file's bytes changed in Explorer (Retry flags `changed since NNN`). */
    changeSketch(file: string) {
      g.sketchVersion[file] = (g.sketchVersion[file] ?? 0) + 1;
    },
    jobs: () => g.jobs,
  };
}
