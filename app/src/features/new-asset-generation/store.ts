// New image tab state and actions (new-asset-generation §6–7) for the open asset project. No
// domain logic: every change is a `gen_*` (or PRV/PRJ/REF) command and the store keeps the
// results. The job, once submitted, belongs to RGN's runner and tracker (FR-GEN-23/25).
import { useSyncExternalStore } from 'react';

import {
  api,
  ApiError,
  onEvent,
  type Capabilities,
  type Connectivity,
  type ControlState,
  type FolderFile,
  type GenDraft,
  type GenEstimate,
  type GenJobDraft,
  type GenSize,
  type GenSizeFit,
  type ModelSummary,
  type ProjectKind,
  type ProjectSnapshot,
  type Spend,
} from '../../api';
import { isFolderMovedError, projectStore } from '../../store/project';
import {
  referencesBlocker,
  referencesCostKey,
  referencesDraft,
  referencesForSend,
  refreshReferences,
  restoreReferences,
  settleReferences,
} from '../references';
import {
  errorText,
  EstimateDebouncer,
  evaluateInput,
  fromEvaluation,
  missingRequired,
  refreshJobs,
  sentValues,
  withinConfirmed,
  withRemembered,
  type ConfirmPlan,
  type UiError,
  type Values,
} from '../region-edit';
import { clampGrid, clampLongSide, longSideLimits, sizeFor, sizeKey } from './size';

export type EstimateStatus = 'idle' | 'waiting' | 'loading' | 'ready' | 'error';

export interface GenState {
  projectId: string | null;
  kind: ProjectKind;
  readOnly: string | null;
  /** The New image tab is shown (FR-GEN-36); estimates run only then. */
  visible: boolean;
  provider: { configured: boolean; connectivity: Connectivity } | null;
  models: ModelSummary[];
  modelsStale: boolean;
  modelsLoading: boolean;
  modelsError: UiError | null;
  modelId: string | null;
  caps: Capabilities | null;
  capsLoading: boolean;
  capsError: UiError | null;
  values: Values;
  touched: string[];
  evaluation: ControlState[] | null;
  notices: string[];
  /** The typed prompt (PRM's prompt block replaces the textarea in slice 6). */
  prompt: string;
  seedMode: 'random' | 'pinned';
  seed: number | null;
  size: GenSize;
  fit: GenSizeFit | null;
  fitError: UiError | null;
  /** The chosen sketch in `structure/`, null = None (FR-GEN-14). */
  sketch: string | null;
  /** Retry's flag on the sketch (FR-GEN-32). */
  sketchFlag: { changed_since?: string | null; missing?: boolean } | null;
  /** `structure/` images, sorted by name; null until listed. */
  sketches: FolderFile[] | null;
  sketchError: UiError | null;
  estimate: {
    status: EstimateStatus;
    result: GenEstimate | null;
    error: UiError | null;
    signature: string | null;
  };
  spend: Spend | null;
  confirmOpen: boolean;
  confirmError: UiError | null;
  submitting: boolean;
  submitError: UiError | null;
  /** One line under the tab's head (Retry filled it, a submit went through). */
  info: string | null;
}

/** FR-PRJ-19: the draft is saved ≥ 2 s after the last change. */
export const DRAFT_SAVE_MS = 2000;

const initial: GenState = {
  projectId: null,
  kind: 'map',
  readOnly: null,
  visible: false,
  provider: null,
  models: [],
  modelsStale: false,
  modelsLoading: false,
  modelsError: null,
  modelId: null,
  caps: null,
  capsLoading: false,
  capsError: null,
  values: {},
  touched: [],
  evaluation: null,
  notices: [],
  prompt: '',
  seedMode: 'random',
  seed: null,
  size: sizeFor('map', null),
  fit: null,
  fitError: null,
  sketch: null,
  sketchFlag: null,
  sketches: null,
  sketchError: null,
  estimate: { status: 'idle', result: null, error: null, signature: null },
  spend: null,
  confirmOpen: false,
  confirmError: null,
  submitting: false,
  submitError: null,
  info: null,
};

let state: GenState = initial;
const listeners = new Set<() => void>();

function set(patch: Partial<GenState> | ((s: GenState) => Partial<GenState>)): void {
  const p = typeof patch === 'function' ? patch(state) : patch;
  state = { ...state, ...p };
  for (const l of listeners) l();
}

const uiError = (e: unknown): UiError =>
  e instanceof ApiError
    ? {
        code: (e.detail as { provider?: { code?: string } } | undefined)?.provider?.code ?? e.code,
        message: e.message,
        ...(e.field ? { field: e.field } : {}),
      }
    : { code: 'ui', message: String(e) };

/** The last value of each async load wins. */
const tokens: Record<string, number> = {};
const nextToken = (k: string) => (tokens[k] = (tokens[k] ?? 0) + 1);
const isCurrent = (k: string, t: number) => tokens[k] === t;

const debouncer = new EstimateDebouncer((sig) => void runEstimate(sig));
let draftTimer: ReturnType<typeof setTimeout> | null = null;
let unsubscribe: (() => void) | null = null;

// ── Derived ──────────────────────────────────────────────────────────────────────────────────

export const isOffline = (s: GenState): boolean =>
  s.modelsStale || s.provider?.connectivity === 'offline';

/** FR-GEN-06: a model without `candidate_count` gives one result per job (R72). */
export const singleResult = (caps: Capabilities): boolean =>
  !caps.controls.some((c) => c.role === 'candidate_count');

/** The sketch is sent in a cost-affecting slot: its choice re-estimates (FR-GEN-20). */
function sketchCostly(s: GenState): boolean {
  const slot = s.caps?.structure?.slot;
  return !!slot && !!s.caps?.slots.find((x) => x.name === slot)?.cost_affecting;
}

/** The sketch block is shown (FR-GEN-14): the model takes one. */
export const takesSketch = (s: GenState): boolean => !!s.caps?.structure;

export const missing = (s: GenState): string[] =>
  s.caps ? missingRequired(s.caps, s.evaluation, s.values, s.prompt) : [];

/** Why Generate is disabled (null = enabled). §7 states, FR-GEN-16/22/34. */
export function submitBlocker(s: GenState, activeJob: string | null): string | null {
  if (!s.projectId) return 'Open an asset project.';
  if (s.readOnly) return 'The project is read-only.';
  if (s.provider && !s.provider.configured) return 'Set up the provider first.';
  if (isOffline(s)) return 'Offline: estimates and Generate need the provider.';
  if (activeJob) return 'A job is running.';
  if (!s.caps) return s.capsLoading || s.modelsLoading ? 'Loading the model…' : 'Pick a model.';
  if (!s.fit)
    return s.fitError ? errorText(s.fitError.code, s.fitError.message) : 'Fitting the size…';
  const refs = referencesBlocker('gen');
  if (refs) return refs;
  const m = missing(s);
  if (m.length) return `Required: ${m.join(', ')}.`;
  if (s.estimate.status === 'error' && s.estimate.error)
    return errorText(s.estimate.error.code, s.estimate.error.message);
  if (s.estimate.status !== 'ready' || !s.estimate.result) return 'Waiting for the estimate…';
  if (s.submitting) return 'Submitting…';
  return null;
}

/** `draft.generate` as saved (FR-GEN-19): only the controls the DM set. */
function storedDraft(s: GenState): GenDraft {
  return {
    model_id: s.modelId,
    controls: Object.fromEntries(s.touched.map((n) => [n, s.values[n]])),
    seed: s.seedMode === 'pinned' ? s.seed : null,
    size: s.size,
    structure: s.sketch ? { file: s.sketch } : null,
    references: referencesDraft('gen').references ?? null,
    typed_prompt: s.prompt,
  };
}

/** The draft estimate and submit send (FR-GEN-22). The final prompt is the typed one until PRM's
 *  prompt block composes it (FR-GEN-16, slice 6). */
export function currentDraft(s: GenState): GenJobDraft | null {
  if (!s.caps || !s.modelId) return null;
  return {
    model_id: s.modelId,
    controls: sentValues(s.caps, s.evaluation, s.values, s.touched),
    seed: s.seedMode === 'pinned' ? s.seed : null,
    size: s.size,
    structure: s.sketch ? { file: s.sketch } : null,
    references: referencesForSend('gen'),
    typed_prompt: s.prompt,
    final_prompt: s.prompt,
  };
}

/** FR-GEN-20: model, sent size or ratio, cost-affecting controls, cost-affecting slots (the
 *  sketch included). The prompt never re-estimates. */
function signatureOf(s: GenState): string | null {
  if (!s.projectId || !s.visible || s.readOnly || isOffline(s)) return null;
  if (!s.caps || !s.modelId) return null;
  const key = sizeKey(s.fit);
  if (!key) return null;
  if (missing(s).some((n) => n !== 'prompt')) return null;
  const controls = sentValues(s.caps, s.evaluation, s.values, s.touched);
  const costly = s.caps.controls
    .filter((c) => c.cost_affecting)
    .map((c) => `${c.name}=${JSON.stringify(controls[c.name] ?? null)}`);
  const sketch = sketchCostly(s) ? (s.sketch ?? '') : '';
  return `${s.modelId}|${key}|${costly.join(',')}|refs:${referencesCostKey('gen')}|sketch:${sketch}`;
}

function schedule(): void {
  const sig = signatureOf(state);
  if (sig === debouncer.signature && sig !== null) return;
  debouncer.update(sig);
  set({
    estimate:
      sig === null
        ? { status: 'idle', result: null, error: null, signature: null }
        : { status: 'waiting', result: null, error: null, signature: sig },
  });
}

async function runEstimate(sig: string): Promise<void> {
  const draft = currentDraft(state);
  if (!draft) return;
  const t = nextToken('estimate');
  set({ estimate: { status: 'loading', result: null, error: null, signature: sig } });
  try {
    const result = await api.gen.estimate(draft);
    if (!isCurrent('estimate', t) || debouncer.signature !== sig) return;
    set({
      estimate: { status: 'ready', result, error: null, signature: sig },
      spend: { spend: result.spend, cap: result.cap, threshold: result.threshold },
    });
  } catch (e) {
    if (!isCurrent('estimate', t)) return;
    if (isFolderMovedError(e)) projectStore.noteFolderMoved();
    const error = uiError(e);
    set({ estimate: { status: 'error', result: null, error, signature: sig } });
    if (error.code === 'reference_unavailable') {
      refreshReferences('gen');
      void loadSketches();
    }
  }
}

function putDraft(): void {
  const s = state;
  if (!s.projectId || s.readOnly) return;
  if (projectStore.get().project?.project_id !== s.projectId) return;
  api.project.putDraft({ generate: storedDraft(s) }).catch((e) => projectStore.draftSaveFailed(e));
}

function saveDraftSoon(): void {
  if (draftTimer) clearTimeout(draftTimer);
  draftTimer = setTimeout(() => {
    draftTimer = null;
    putDraft();
  }, DRAFT_SAVE_MS);
}

// ── Loads ────────────────────────────────────────────────────────────────────────────────────

async function refreshProvider(): Promise<void> {
  try {
    const st = await api.provider.status();
    set({ provider: { configured: st.configured, connectivity: st.connectivity } });
  } catch {
    // Unknown status: the estimate's errors say what is wrong.
  }
  schedule();
}

async function loadSpend(): Promise<void> {
  try {
    set({ spend: await api.region.getSpend() });
  } catch {
    // The confirm asks again.
  }
}

/** FR-GEN-14: `structure/` images (FR-REF-01 rules), sorted by name. */
async function loadSketches(): Promise<void> {
  const pid = state.projectId;
  if (!pid) return;
  const t = nextToken('sketches');
  try {
    const files = await api.project.folderList('structure');
    if (!isCurrent('sketches', t) || state.projectId !== pid) return;
    const sketches = files.filter((f) => f.image).sort((a, b) => a.name.localeCompare(b.name));
    set({ sketches, sketchError: null });
  } catch (e) {
    if (!isCurrent('sketches', t)) return;
    set({ sketches: [], sketchError: uiError(e) });
  }
}

async function loadModels(remembered: Values | null): Promise<void> {
  set({ modelsLoading: true, modelsError: null });
  try {
    const { models, stale } = await api.provider.listModels('generate');
    set({ models, modelsStale: stale, modelsLoading: false });
    const keep = state.modelId && models.some((m) => m.id === state.modelId) ? state.modelId : null;
    if (state.modelId && !keep)
      set((cur) => ({
        notices: [...cur.notices, 'The saved model isn’t available; the first one is selected.'],
      }));
    const id = keep ?? models[0]?.id ?? null;
    if (id) await selectModel(id, remembered);
    else set({ modelId: null, caps: null, evaluation: null, fit: null });
  } catch (e) {
    set({ modelsLoading: false, modelsError: uiError(e) });
  }
  schedule();
}

/** OQ-GEN-07 / OQ-GEN-11: Tokens default to a transparent background where the model allows
 *  it, unless the DM chose a background. */
function withKindDefaults(
  caps: Capabilities,
  kind: ProjectKind,
  r: { values: Values; touched: string[] },
): { values: Values; touched: string[] } {
  const bg = caps.controls.find((c) => c.role === 'background');
  if (kind !== 'token' || !bg || r.touched.includes(bg.name)) return r;
  if (!bg.allowed.includes('transparent')) return r;
  return { values: { ...r.values, [bg.name]: 'transparent' }, touched: [...r.touched, bg.name] };
}

async function selectModel(id: string, remembered: Values | null): Promise<void> {
  const t = nextToken('caps');
  set({ modelId: id, capsLoading: true, capsError: null, evaluation: null, fit: null });
  try {
    const caps = await api.provider.getCapabilities(id, 'generate');
    if (!isCurrent('caps', t)) return;
    const r = withKindDefaults(caps, state.kind, withRemembered(caps, remembered));
    // A wh model keeps the long side within its limits (FR-GEN-10).
    const lim = longSideLimits(caps);
    const size =
      state.size.mode === 'ratio' && lim
        ? { ...state.size, long_side: clampLongSide(state.size.long_side, lim) }
        : state.size;
    set({ caps, capsLoading: false, values: r.values, touched: r.touched, size });
    await Promise.all([evaluate(), refit()]);
  } catch (e) {
    if (!isCurrent('caps', t)) return;
    set({ caps: null, capsLoading: false, capsError: uiError(e) });
    schedule();
  }
}

async function evaluate(): Promise<void> {
  const s = state;
  if (!s.caps || !s.modelId) return;
  const t = nextToken('evaluate');
  try {
    const r = await api.gen.evaluateControls(
      s.modelId,
      evaluateInput(s.caps, s.values, s.prompt, s.seedMode === 'pinned' ? s.seed : null),
    );
    if (!isCurrent('evaluate', t) || state.modelId !== s.modelId) return;
    // FR-GEN-19 / FR-RGN-11: a rejected value resets to its default with a notice.
    const { values, touched, notices } = fromEvaluation(s.caps, state.values, state.touched, r);
    set((cur) => ({
      evaluation: r.controls,
      values,
      touched,
      notices: notices.length ? [...cur.notices, ...notices] : cur.notices,
    }));
  } catch (e) {
    if (!isCurrent('evaluate', t)) return;
    set({ capsError: uiError(e) });
  }
  schedule();
}

/** FR-GEN-08/09: the core fits the asked size to the model. */
async function refit(): Promise<void> {
  const s = state;
  if (!s.modelId || !s.caps) {
    set({ fit: null, fitError: null });
    schedule();
    return;
  }
  const t = nextToken('fit');
  try {
    const fit = await api.gen.sizeFit(s.modelId, s.kind, s.size);
    if (!isCurrent('fit', t)) return;
    set({ fit, fitError: null });
  } catch (e) {
    if (!isCurrent('fit', t)) return;
    set({ fit: null, fitError: uiError(e) });
  }
  schedule();
}

async function onEventSafe<E extends Parameters<typeof onEvent>[0]>(
  name: E,
  handler: Parameters<typeof onEvent<E>>[1],
): Promise<() => void> {
  try {
    return await onEvent(name, handler);
  } catch {
    return () => undefined;
  }
}

function subscribe(): void {
  if (unsubscribe) return;
  const subs = [
    onEventSafe('provider:status-changed', () => void refreshProvider()),
    onEventSafe('rgn:spend', (p) =>
      set((cur) => ({
        spend: { spend: p.spend, cap: p.cap, threshold: cur.spend?.threshold ?? null },
      })),
    ),
  ];
  // The project closed or another one opened, maybe while this tab was hidden: let it go.
  const offProject = projectStore.subscribe(() => {
    const p = projectStore.get().project;
    if (state.projectId && p?.project_id !== state.projectId) gen.detach();
  });
  unsubscribe = () => {
    offProject();
    subs.forEach((p) => void p.then((u) => u()).catch(() => undefined));
  };
}

// ── Actions ──────────────────────────────────────────────────────────────────────────────────

export const gen = {
  get: (): GenState => state,
  subscribe(l: () => void): () => void {
    listeners.add(l);
    return () => listeners.delete(l);
  },

  /** Binds to the open project (idempotent per project id). */
  async attach(project: ProjectSnapshot): Promise<void> {
    if (state.projectId === project.project_id) {
      if (state.readOnly !== project.read_only) {
        set({ readOnly: project.read_only });
        schedule();
      }
      return;
    }
    gen.detach();
    const d = project.draft?.generate ?? null;
    set({
      projectId: project.project_id,
      kind: project.kind,
      readOnly: project.read_only,
      modelId: d?.model_id ?? null,
      prompt: d?.typed_prompt ?? '',
      seedMode: d?.seed !== null && d?.seed !== undefined ? 'pinned' : 'random',
      seed: d?.seed ?? null,
      size: sizeFor(project.kind, d?.size),
      sketch: d?.structure?.file ?? null,
    });
    subscribe();
    await Promise.all([
      refreshProvider(),
      loadSpend(),
      loadSketches(),
      loadModels(d?.controls ?? null),
    ]);
  },

  detach(): void {
    unsubscribe?.();
    unsubscribe = null;
    debouncer.reset();
    // As RGN: an edit still waiting for its save goes with the project (never into another).
    if (draftTimer) clearTimeout(draftTimer);
    draftTimer = null;
    for (const k of Object.keys(tokens)) nextToken(k);
    state = initial;
    for (const l of listeners) l();
  },

  /** The tab was shown or hidden (FR-GEN-36). */
  setVisible(visible: boolean): void {
    if (visible === state.visible) return;
    set({ visible });
    if (!visible) debouncer.reset();
    schedule();
  },

  /** The References block changed (FR-GEN-13): re-render submit; save a user edit. */
  referencesChanged(save: boolean): void {
    set({});
    schedule();
    if (save) saveDraftSoon();
  },

  pickModel(id: string): Promise<void> {
    const keep = Object.fromEntries(state.touched.map((n) => [n, state.values[n]]));
    saveDraftSoon();
    return selectModel(id, keep);
  },

  setValue(name: string, value: unknown): void {
    set((s) => ({
      values: { ...s.values, [name]: value },
      touched: s.touched.includes(name) ? s.touched : [...s.touched, name],
    }));
    saveDraftSoon();
    schedule();
    void evaluate();
  },

  setPrompt(prompt: string): void {
    set({ prompt });
    saveDraftSoon();
    schedule();
  },

  setSeedMode(mode: 'random' | 'pinned'): void {
    set((s) => ({ seedMode: mode, seed: mode === 'pinned' ? (s.seed ?? 0) : s.seed }));
    saveDraftSoon();
  },

  setSeed(seed: number | null): void {
    set({ seed });
    saveDraftSoon();
  },

  /** FR-GEN-07: cols, rows, px per square (Maps). */
  setGrid(patch: Partial<{ cols: number; rows: number; pps: number }>): void {
    const s = state.size;
    if (s.mode !== 'grid') return;
    set({ size: { mode: 'grid', ...clampGrid({ ...s, ...patch }) } });
    saveDraftSoon();
    void refit();
  },

  /** FR-GEN-10: ratio and long side (Tokens, Items). */
  setRatio(patch: Partial<{ aspect: string; long_side: number }>): void {
    const s = state.size;
    if (s.mode !== 'ratio') return;
    const lim = longSideLimits(state.caps);
    const next = { ...s, ...patch };
    if (lim) next.long_side = clampLongSide(next.long_side, lim);
    set({ size: next });
    saveDraftSoon();
    void refit();
  },

  /** FR-GEN-14: one sketch or None. Changing it clears a Retry flag. */
  setSketch(file: string | null): void {
    set({ sketch: file, sketchFlag: null });
    saveDraftSoon();
    schedule();
  },

  /** FR-GEN-14 Add…: copies files into `structure/` (`project_folder_add`, #53). */
  async addSketches(paths: string[]): Promise<void> {
    if (!paths.length) return;
    try {
      await api.project.folderAdd('structure', paths);
      set({ sketchError: null });
    } catch (e) {
      set({ sketchError: uiError(e) });
    }
    await loadSketches();
  },

  dismissNotices(): void {
    set({ notices: [], info: null });
  },

  async openConfirm(activeJob: string | null): Promise<void> {
    if (submitBlocker(state, activeJob)) return;
    await loadSpend();
    set({ confirmOpen: true, confirmError: null, submitError: null });
  },

  closeConfirm(): void {
    set({ confirmOpen: false, confirmError: null });
  },

  /** Confirm OK (FR-GEN-21/22): store the spend settings if asked, then submit. */
  async confirm(plan: ConfirmPlan): Promise<void> {
    const draft = currentDraft(state);
    const est = state.estimate.result;
    if (!draft || !est || !plan.canConfirm) return;
    set({ submitting: true, confirmError: null });
    try {
      if (plan.saveSpend) set({ spend: await api.region.setSpend(plan.saveSpend) });
      const submit = (estimateId: string) =>
        api.gen.submit({ draft, estimate_id: estimateId, confirmations: plan.confirmations });
      try {
        await submit(est.estimate_id);
      } catch (e) {
        // BL-36: the estimate expired while the dialog was open.
        if (uiError(e).code !== 'estimate_stale') throw e;
        const fresh = await api.gen.estimate(draft);
        set({
          estimate: { status: 'ready', result: fresh, error: null, signature: debouncer.signature },
          spend: { spend: fresh.spend, cap: fresh.cap, threshold: fresh.threshold },
        });
        if (!withinConfirmed(fresh, plan)) {
          set({
            submitting: false,
            confirmError: {
              code: 'estimate_changed',
              message:
                'The estimate changed while this dialog was open. Check it and confirm again.',
            },
          });
          return;
        }
        await submit(fresh.estimate_id);
      }
      set({ submitting: false, confirmOpen: false, sketchFlag: null, info: null });
      settleReferences('gen');
      await Promise.all([refreshJobs(), loadSpend()]);
    } catch (e) {
      const err = uiError(e);
      if (err.code === 'over_spend_cap' || err.code === 'confirmation_missing') {
        await loadSpend();
        set({ submitting: false, confirmError: err });
        return;
      }
      set({ submitting: false, confirmOpen: false, submitError: err });
      if (isFolderMovedError(e)) projectStore.noteFolderMoved();
      if (err.code === 'estimate_stale') debouncer.force();
      if (err.code === 'reference_unavailable') {
        refreshReferences('gen');
        void loadSketches();
      }
      void refreshJobs();
    }
  },

  /** FR-GEN-32: pre-fill the tab from a generate job, then estimate → confirm again. */
  async retry(jobId: string): Promise<void> {
    const project = projectStore.get().project;
    if (project) await gen.attach(project);
    try {
      const d = await api.gen.retryDraft(jobId);
      const flag = d.structure?.changed_since || d.structure?.missing ? d.structure : null;
      set({
        prompt: d.typed_prompt,
        seedMode: d.seed === null ? 'random' : 'pinned',
        seed: d.seed,
        size: d.size ? sizeFor(state.kind, d.size) : state.size,
        sketch: d.structure?.file ?? null,
        sketchFlag: flag ? { changed_since: flag.changed_since, missing: flag.missing } : null,
        submitError: null,
        info: 'The tab is filled from the job. Check it, then generate.',
      });
      debouncer.reset();
      restoreReferences(d.references, 'gen'); // FR-REF-26 flags (REF resolves them)
      if (d.model_id) await selectModel(d.model_id, d.controls);
      saveDraftSoon();
    } catch (e) {
      const err = uiError(e);
      set({ info: null, submitError: err });
    }
  },
};

export function useGen<T>(select: (s: GenState) => T): T {
  return useSyncExternalStore(
    gen.subscribe,
    () => select(state),
    () => select(state),
  );
}
