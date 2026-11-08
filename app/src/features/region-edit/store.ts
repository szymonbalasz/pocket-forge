// Region-edit UI state and actions for the open asset project. No domain logic: every change is
// an `rgn_*` (or PRV/PRJ) command and the store keeps the results. Shared by the Generate panel,
// the iteration tracker and the status-bar segment. Drives EDT's picker and compare through the
// editor bridge (R16, R19) and pushes accept / re-tune onto EDT's undo stack (FR-RGN-30).
import { useSyncExternalStore } from 'react';

import {
  api,
  ApiError,
  onEvent,
  type Capabilities,
  type CandidateRecord,
  type Connectivity,
  type CostValue,
  type JobState,
  type Layer,
  type ModelSummary,
  type ProjectSnapshot,
  type RectPx,
  type RegionFit,
  type RgnBlend,
  type ControlState,
  type JobDraft,
  type JobEstimate,
  type RgnIteration,
  type RgnPreview,
  type Spend,
} from '../../api';
import { projectStore } from '../../store/project';
import {
  editorBridge,
  flushMask,
  pushUndo,
  revisionUndoEntry,
  thumbnailUrl,
  type PickerCandidate,
} from '../editor';
import {
  referencesBlocker,
  referencesCostKey,
  referencesDraft,
  referencesForSend,
  refreshReferences,
  reloadReferences,
  restoreReferences,
  settleReferences,
} from '../references';
import type { ConfirmPlan } from './confirm';
import {
  costSignature,
  evaluateInput,
  missingRequired,
  fromEvaluation,
  withRemembered,
  sentValues,
  type Values,
} from './controls';
import { EstimateDebouncer } from './estimate';
import { errorText } from './format';
import { defaultSelection, isActive, iterationCandidates, orderRows } from './tracker';

export interface UiError {
  code: string;
  message: string;
  field?: string;
}

export interface LiveJob {
  state: JobState;
  progress: number | null;
  cost: CostValue | null;
  error: { code: string; hint?: string; message?: string } | null;
  longRunning: boolean;
  /** When this session first saw the job running (for the elapsed time). */
  startedAt: number;
}

export type EstimateStatus = 'idle' | 'waiting' | 'loading' | 'ready' | 'error';

export interface RgnState {
  projectId: string | null;
  readOnly: string | null;
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
  prompt: string;
  seedMode: 'random' | 'pinned';
  seed: number | null;
  margin: number;
  /** The region's inner rect: EDT's (R18), or a retried job's until the region changes. */
  inner: RectPx | null;
  maskId: string | null;
  fit: RegionFit | null;
  fitError: UiError | null;
  estimate: {
    status: EstimateStatus;
    result: JobEstimate | null;
    error: UiError | null;
    signature: string | null;
  };
  spend: Spend | null;
  confirmOpen: boolean;
  confirmError: UiError | null;
  submitting: boolean;
  submitError: UiError | null;
  iterations: RgnIteration[];
  selectedJobId: string | null;
  live: Record<string, LiveJob>;
  candidates: CandidateRecord[];
  layers: Layer[];
  headRevision: string | null;
  selectedCandidateId: string | null;
  preview: RgnPreview | null;
  blend: RgnBlend;
  trackerNotice: { kind: 'info' | 'error'; text: string } | null;
  thumbs: Record<string, string>;
}

export const DEFAULT_MARGIN = 128;
export const DEFAULT_BLEND: RgnBlend = { feather_px: 48, colour_match: true };
/** FR-PRJ-19: the draft is saved ≥ 2 s after the last change. */
export const DRAFT_SAVE_MS = 2000;
const PREVIEW_DEBOUNCE_MS = 150;

const initial: RgnState = {
  projectId: null,
  readOnly: null,
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
  margin: DEFAULT_MARGIN,
  inner: null,
  maskId: null,
  fit: null,
  fitError: null,
  estimate: { status: 'idle', result: null, error: null, signature: null },
  spend: null,
  confirmOpen: false,
  confirmError: null,
  submitting: false,
  submitError: null,
  iterations: [],
  selectedJobId: null,
  live: {},
  candidates: [],
  layers: [],
  headRevision: null,
  selectedCandidateId: null,
  preview: null,
  blend: DEFAULT_BLEND,
  trackerNotice: null,
  thumbs: {},
};

let state: RgnState = initial;
const listeners = new Set<() => void>();

function set(patch: Partial<RgnState> | ((s: RgnState) => Partial<RgnState>)): void {
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

/** The last value of each async load wins: a token per kind drops stale answers. */
const tokens: Record<string, number> = {};
const nextToken = (k: string) => (tokens[k] = (tokens[k] ?? 0) + 1);
const isCurrent = (k: string, t: number) => tokens[k] === t;

const debouncer = new EstimateDebouncer((sig) => void runEstimate(sig));
let draftTimer: ReturnType<typeof setTimeout> | null = null;
let previewTimer: ReturnType<typeof setTimeout> | null = null;
let unsubscribe: (() => void) | null = null;
/** Candidate ids whose thumbnail (FR-EDT-32) is being fetched. */
const thumbsInFlight = new Set<string>();
/** EDT's region inner as last seen, so a retried region is kept until EDT's changes. */
let editorInner: RectPx | null = null;
/** EDT's restored region was taken for this attach (`editorRegion`); later changes are edits. */
let regionTaken = false;

const sameRect = (a: RectPx | null, b: RectPx | null) =>
  a === b || (!!a && !!b && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h);

// ── Derived ──────────────────────────────────────────────────────────────────────────────────

/** Offline: PRV returned cached models (§7 *offline*), or the provider says so. */
export const isOffline = (s: RgnState): boolean =>
  s.modelsStale || s.provider?.connectivity === 'offline';

export const activeJobId = (s: RgnState): string | null =>
  orderRows(s.iterations).find((r) => isActive(r.state))?.job_id ?? null;

export const missing = (s: RgnState): string[] =>
  s.caps ? missingRequired(s.caps, s.evaluation, s.values, s.prompt) : [];

/** Why submit is disabled (null = enabled). FR-RGN-09/17/33/38, §7 states. */
export function submitBlocker(s: RgnState): string | null {
  if (!s.projectId) return 'Open an asset project.';
  if (s.readOnly) return 'The project is read-only.';
  if (s.provider && !s.provider.configured) return 'Set up the provider first.';
  if (isOffline(s)) return 'Offline: estimates and submit need the provider.';
  if (activeJobId(s)) return 'A job is running.';
  if (!s.inner) return 'Drag a box to start a region edit.';
  if (!s.fit)
    return s.fitError ? errorText(s.fitError.code, s.fitError.message) : 'Fitting the region…';
  if (!s.caps) return 'Pick a model.';
  if (!s.maskId) return 'Paint a mask inside the region first.';
  const refs = referencesBlocker(); // FR-REF-18: a slot under its min
  if (refs) return refs;
  const m = missing(s);
  if (m.length) return `Required: ${m.join(', ')}.`;
  if (s.estimate.status === 'error' && s.estimate.error)
    return errorText(s.estimate.error.code, s.estimate.error.message);
  if (s.estimate.status !== 'ready' || !s.estimate.result) return 'Waiting for the estimate…';
  if (s.submitting) return 'Submitting…';
  return null;
}

export function currentDraft(s: RgnState): JobDraft | null {
  if (!s.fit || !s.caps || !s.modelId || !s.maskId) return null;
  return {
    region: { inner: s.fit.inner, margin: s.margin },
    mask_id: s.maskId,
    model_id: s.modelId,
    final_prompt: s.prompt,
    controls: sentValues(s.caps, s.evaluation, s.values, s.touched),
    seed: s.seedMode === 'pinned' ? s.seed : null,
    references: referencesForSend(), // REF's selection (FR-RGN-41)
  };
}

function signatureOf(s: RgnState): string | null {
  if (!s.projectId || s.readOnly || isOffline(s) || !s.fit || !s.caps || !s.modelId || !s.maskId)
    return null;
  // Required controls (not the prompt) must be filled before an estimate makes sense.
  if (missing(s).some((n) => n !== 'prompt')) return null;
  const controls = sentValues(s.caps, s.evaluation, s.values, s.touched);
  const sig = costSignature(s.modelId, { w: s.fit.sent_w, h: s.fit.sent_h }, s.caps, controls);
  // FR-RGN-42: ticks in cost-affecting reference slots re-estimate too.
  return `${sig}|refs:${referencesCostKey()}`;
}

/** FR-RGN-12: re-checks the cost signature; a change re-estimates after 500 ms. */
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
    const result = await api.region.estimate(state.projectId!, draft);
    if (!isCurrent('estimate', t) || debouncer.signature !== sig) return;
    set({
      estimate: { status: 'ready', result, error: null, signature: sig },
      spend: { spend: result.spend, cap: result.cap, threshold: result.threshold },
    });
  } catch (e) {
    if (!isCurrent('estimate', t)) return;
    const error = uiError(e);
    set({ estimate: { status: 'error', result: null, error, signature: sig } });
    if (error.code === 'reference_unavailable') refreshReferences(); // FR-REF-21
  }
}

function saveDraftSoon(): void {
  if (draftTimer) clearTimeout(draftTimer);
  draftTimer = setTimeout(() => {
    draftTimer = null;
    const s = state;
    if (!s.projectId || s.readOnly) return;
    api.project
      .putDraft({
        final_prompt: s.prompt,
        model_id: s.modelId,
        controls: Object.fromEntries(s.touched.map((n) => [n, s.values[n]])),
        seed: s.seedMode === 'pinned' ? s.seed : null,
        region: s.inner ? { inner: s.inner, margin: s.margin } : null,
        ...referencesDraft(), // REF's selection (FR-REF-20)
      })
      // Shown in the save segment (walk D3); the next change saves again.
      .catch((e) => projectStore.draftSaveFailed(e));
  }, DRAFT_SAVE_MS);
}

// ── Loads ────────────────────────────────────────────────────────────────────────────────────

async function refreshProvider(): Promise<void> {
  try {
    const st = await api.provider.status();
    set({ provider: { configured: st.configured, connectivity: st.connectivity } });
  } catch {
    // Unknown provider status: leave submit to the estimate's errors.
  }
  schedule();
}

async function loadSpend(): Promise<void> {
  try {
    set({ spend: await api.region.getSpend() });
  } catch {
    // The confirm dialog asks again (FR-RGN-16).
  }
}

async function refreshProject(): Promise<void> {
  const pid = state.projectId;
  if (!pid) return;
  try {
    const p = await api.project.get();
    if (!p || p.project_id !== state.projectId) return;
    const extra = p as ProjectSnapshot & { candidates?: CandidateRecord[] };
    // A restore (History, or undo of an accept) brings back that revision's draft (FR-PRJ-23).
    const restored =
      p.head_revision !== state.headRevision &&
      p.revisions.find((r) => r.id === p.head_revision)?.reason === 'restore';
    set({
      candidates: extra.candidates ?? [],
      layers: p.layers,
      headRevision: p.head_revision,
      readOnly: p.read_only,
      maskId:
        state.inner && !sameRect(state.inner, editorInner)
          ? state.maskId
          : (p.draft?.mask_id ?? state.maskId),
    });
    if (restored) applyRestoredDraft(p.draft);
    syncPicker();
  } catch {
    // Keep the last snapshot; the next commit event retries.
  }
}

/** Shows a restored revision's draft (walk D4), or the kept version's after a conflict is
 *  resolved (walk 2 N1). No draft save: the core already wrote it, and a save still waiting
 *  would write the replaced draft back. BL-26: its region goes back to EDT, validated (`refit` →
 *  `editorBridge.setRegion`); EDT keeps the overlapping mask and hands it off again. */
function applyRestoredDraft(d: ProjectSnapshot['draft']): void {
  if (draftTimer) clearTimeout(draftTimer);
  draftTimer = null;
  reloadReferences(d?.references ?? null);
  const region = d?.region as { inner?: RectPx; margin?: number } | null | undefined;
  const inner = region?.inner ?? null;
  const margin = region?.margin ?? state.margin;
  if (!sameRect(inner, state.inner) || margin !== state.margin) {
    // EDT's echo of this region is not an edit (no draft save, no prompt reset).
    editorInner = inner;
    set({ inner, margin });
    if (!inner) editorBridge.setRegion(null);
    void refit();
  }
  set({
    prompt: d?.final_prompt ?? '',
    seedMode: d?.seed !== null && d?.seed !== undefined ? 'pinned' : 'random',
    seed: d?.seed ?? null,
  });
  const model = d?.model_id ?? state.modelId;
  if (model) void selectModel(model, d?.controls ?? null);
  else schedule();
}

async function refreshIterations(): Promise<void> {
  if (!state.projectId) return;
  const t = nextToken('iterations');
  try {
    const rows = await api.region.iterations();
    if (!isCurrent('iterations', t)) return;
    const before = orderRows(state.iterations)[0]?.job_id ?? null;
    const newest = orderRows(rows)[0]?.job_id ?? null;
    // A new job selects itself; otherwise keep the user's pick (FR-RGN-40).
    const selected = newest !== before ? newest : defaultSelection(rows, state.selectedJobId);
    const changed = selected !== state.selectedJobId;
    const live = { ...state.live };
    for (const r of rows)
      if (!live[r.job_id] && isActive(r.state))
        live[r.job_id] = {
          state: r.state,
          progress: null,
          cost: r.cost,
          error: null,
          longRunning: false,
          startedAt: Date.now(),
        };
    set({
      iterations: rows,
      selectedJobId: selected,
      live,
      ...(changed ? { selectedCandidateId: null, preview: null } : {}),
    });
    if (changed) editorBridge.setCompare(null);
    syncPicker();
    schedule();
  } catch {
    // `no_project` or a transient error: the next event retries.
  }
}

async function loadModels(remembered: Values | null): Promise<void> {
  set({ modelsLoading: true, modelsError: null });
  try {
    const { models, stale } = await api.provider.listModels('edit_masked');
    set({ models, modelsStale: stale, modelsLoading: false });
    const keep = state.modelId && models.some((m) => m.id === state.modelId) ? state.modelId : null;
    const id = keep ?? models[0]?.id ?? null;
    if (id) await selectModel(id, remembered);
    else set({ modelId: null, caps: null, evaluation: null });
  } catch (e) {
    set({ modelsLoading: false, modelsError: uiError(e) });
  }
}

async function evaluate(): Promise<void> {
  const s = state;
  if (!s.caps || !s.modelId) return;
  const t = nextToken('evaluate');
  try {
    const r = await api.region.evaluateControls(
      s.modelId,
      evaluateInput(s.caps, s.values, s.prompt, s.seedMode === 'pinned' ? s.seed : null),
    );
    // The core resets and re-evaluates itself (FR-RGN-33); one call is enough.
    const { values, touched, notices } = fromEvaluation(s.caps, s.values, s.touched, r);
    const evaluation = r.controls;
    if (!isCurrent('evaluate', t) || state.modelId !== s.modelId) return;
    set((cur) => ({
      evaluation,
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

async function refit(): Promise<void> {
  const s = state;
  if (!s.projectId || !s.inner) {
    set({ fit: null, fitError: null });
    schedule();
    return;
  }
  const t = nextToken('fit');
  try {
    const fit = await api.region.fit(s.projectId, s.inner, s.margin, s.modelId ?? undefined);
    if (!isCurrent('fit', t)) return;
    set({ fit, fitError: null });
    // BL-14: EDT draws the region RGN validated (margin, model, Retry's region).
    editorBridge.setRegion({ inner: fit.inner, outer: fit.outer });
  } catch (e) {
    if (!isCurrent('fit', t)) return;
    set({ fit: null, fitError: uiError(e) });
  }
  schedule();
}

// ── Picker (R19) ─────────────────────────────────────────────────────────────────────────────

const pickerHandlers = {
  onSelect: (id: string) => void rgn.selectCandidate(id),
  onAccept: (id: string) => void rgn.accept(id),
  onStar: (id: string, starred: boolean) => void rgn.star(id, starred),
  onDiscard: (id: string) => void rgn.discard(id),
};

function syncPicker(): void {
  const s = state;
  const jobId = s.selectedJobId;
  const list = jobId ? iterationCandidates(s.candidates, jobId) : [];
  if (!s.projectId || list.length === 0) {
    editorBridge.setCandidates(null);
    return;
  }
  const candidates: PickerCandidate[] = list.map((c) => ({
    id: c.id,
    index: c.index,
    status: c.status,
    starred: c.starred,
    thumbUrl: s.thumbs[c.id] ?? null,
    preview:
      c.id === s.selectedCandidateId && s.preview
        ? { url: s.preview.url, rect: s.preview.rect }
        : null,
  }));
  const selected = list.some((c) => c.id === s.selectedCandidateId) ? s.selectedCandidateId : null;
  editorBridge.setCandidates(candidates, pickerHandlers, selected);
  const missingThumbs = list.filter(
    (c) => c.status === 'ready' && !s.thumbs[c.id] && !thumbsInFlight.has(c.id),
  );
  const pid = s.projectId;
  for (const c of missingThumbs) {
    thumbsInFlight.add(c.id);
    thumbnailUrl(pid, c.id)
      .then(
        (url) => {
          if (state.projectId !== pid) return;
          set((cur) => ({ thumbs: { ...cur.thumbs, [c.id]: url } }));
          syncPicker();
        },
        () => undefined,
      )
      .finally(() => thumbsInFlight.delete(c.id));
  }
}

async function previewSelected(): Promise<void> {
  const s = state;
  const jobId = s.selectedJobId;
  const candId = s.selectedCandidateId;
  if (!jobId || !candId) return;
  const t = nextToken('preview');
  try {
    const p = await api.region.preview({ job_id: jobId, candidate_id: candId, blend: s.blend });
    if (!isCurrent('preview', t) || state.selectedCandidateId !== candId) return;
    set({ preview: p });
    if (p.notices?.includes('colour_match_skipped')) {
      set({
        trackerNotice: {
          kind: 'info',
          text: 'Colour match skipped: less than 1 % of the region is unmasked.',
        },
      });
    }
    editorBridge.setCompare({ url: p.url, rect: p.rect });
  } catch (e) {
    if (!isCurrent('preview', t)) return;
    const err = uiError(e);
    set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
  }
}

const layerOf = (s: RgnState, candidateId: string) =>
  (s.layers as (Layer & { candidate_id?: string })[]).find(
    (l) => l.kind === 'patch' && l.candidate_id === candidateId,
  );

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
    onEventSafe('rgn:job-state', (p) => {
      const prev = state.live[p.job_id];
      set((cur) => ({
        live: {
          ...cur.live,
          [p.job_id]: {
            state: p.state,
            progress: p.progress,
            cost: p.cost,
            error: p.error ?? null,
            longRunning: p.long_running ?? prev?.longRunning ?? false,
            startedAt: prev?.startedAt ?? Date.now(),
          },
        },
      }));
      if (!prev || prev.state !== p.state) void refreshIterations();
      if (p.state === 'fetched' || p.state === 'closed') void refreshProject();
    }),
    onEventSafe('rgn:candidate-ready', () => void refreshProject()),
    onEventSafe('rgn:spend', (p) =>
      set((cur) => ({
        spend: { spend: p.spend, cap: p.cap, threshold: cur.spend?.threshold ?? null },
      })),
    ),
    onEventSafe('project:committed', () => {
      void refreshProject();
      void refreshIterations();
    }),
    onEventSafe('provider:status-changed', () => void refreshProvider()),
    Promise.resolve(
      // Keep other (walk 2 N1): the kept draft. Keep this: the panel is this version already
      // (the core saved the refused draft; a save still waiting now succeeds).
      projectStore.onResolved((p, keep) => {
        if (p.project_id !== state.projectId || keep !== 'other') return;
        set({ headRevision: p.head_revision, maskId: p.draft?.mask_id ?? null });
        applyRestoredDraft(p.draft);
      }),
    ),
  ];
  unsubscribe = () =>
    subs.forEach(
      (p) =>
        void p
          .then((u) => u())
          // The core went away first (app close, test teardown).
          .catch(() => undefined),
    );
}

async function selectModel(id: string, remembered: Values | null): Promise<void> {
  const t = nextToken('caps');
  set({ modelId: id, capsLoading: true, capsError: null, evaluation: null });
  try {
    const caps = await api.provider.getCapabilities(id);
    if (!isCurrent('caps', t)) return;
    const r = withRemembered(caps, remembered);
    set({ caps, capsLoading: false, values: r.values, touched: r.touched, notices: r.notices });
    await Promise.all([evaluate(), refit()]);
  } catch (e) {
    if (!isCurrent('caps', t)) return;
    set({ caps: null, capsLoading: false, capsError: uiError(e) });
    schedule();
  }
}

// ── Actions ──────────────────────────────────────────────────────────────────────────────────

export const rgn = {
  get: (): RgnState => state,
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
    rgn.detach();
    const d = project.draft;
    const region = d?.region as { inner?: RectPx; margin?: number } | null | undefined;
    set({
      projectId: project.project_id,
      readOnly: project.read_only,
      // Opening never writes the draft (walk D2): the saved region stands until EDT restores it.
      inner: region?.inner ?? null,
      prompt: d?.final_prompt ?? '',
      seedMode: d?.seed !== null && d?.seed !== undefined ? 'pinned' : 'random',
      seed: d?.seed ?? null,
      margin: region?.margin ?? DEFAULT_MARGIN,
      maskId: d?.mask_id ?? null,
      modelId: d?.model_id ?? null,
      layers: project.layers,
      headRevision: project.head_revision,
    });
    subscribe();
    await Promise.all([
      refreshProvider(),
      loadSpend(),
      refreshIterations(),
      refreshProject(),
      loadModels(d?.controls ?? null),
    ]);
  },

  /** The References block changed (REF, FR-REF-20): re-render submit; save a user edit. */
  referencesChanged(save: boolean): void {
    set({});
    schedule();
    if (save) saveDraftSoon();
  },

  detach(): void {
    unsubscribe?.();
    unsubscribe = null;
    debouncer.reset();
    if (draftTimer) clearTimeout(draftTimer);
    if (previewTimer) clearTimeout(previewTimer);
    draftTimer = previewTimer = null;
    editorInner = null;
    regionTaken = false;
    thumbsInFlight.clear();
    for (const k of Object.keys(tokens)) nextToken(k);
    if (state.projectId) editorBridge.setCandidates(null);
    state = initial;
    for (const l of listeners) l();
  },

  /** EDT's region for the attached project, once EDT has restored it (FR-EDT-19). The first value
   *  is the reopened region: taken without a draft save or a prompt reset (walk D2); every later
   *  one is an edit (`setRegion`). */
  editorRegion(inner: RectPx | null): void {
    if (regionTaken) return rgn.setRegion(inner);
    regionTaken = true;
    editorInner = inner;
    set({ inner });
    void refit();
  },

  /** EDT's validated region changed (R18). Null = the region was removed. */
  setRegion(inner: RectPx | null): void {
    if (sameRect(inner, editorInner)) return;
    editorInner = inner;
    // A new region starts with an empty prompt (FR-RGN-11): removing it clears the prompt.
    set(inner ? { inner } : { inner: null, prompt: '' });
    saveDraftSoon();
    void refit();
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
    // FR-RGN-12: the prompt never re-estimates; schedule() only re-checks "required".
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

  setMargin(margin: number): void {
    set({ margin: Math.max(0, Math.min(512, Math.round(margin))) });
    saveDraftSoon();
    void refit();
  },

  dismissNotices(): void {
    set({ notices: [] });
  },

  async openConfirm(): Promise<void> {
    if (submitBlocker(state)) return;
    await loadSpend();
    set({ confirmOpen: true, confirmError: null, submitError: null });
  },

  closeConfirm(): void {
    set({ confirmOpen: false, confirmError: null });
  },

  /** Confirm-dialog OK: store the spend settings if asked, then submit (FR-RGN-13..17). */
  async confirm(plan: ConfirmPlan): Promise<void> {
    const s = state;
    const draft = currentDraft(s);
    const est = s.estimate.result;
    if (!draft || !est || !plan.canConfirm) return;
    set({ submitting: true, confirmError: null });
    try {
      if (plan.saveSpend) {
        const spend = await api.region.setSpend(plan.saveSpend);
        set({ spend });
      }
      // R7: EDT submits the live mask first, so the job sends exactly what the DM sees.
      const ref = await flushMask();
      if (ref) set({ maskId: ref.mask_id });
      const { job_id } = await api.region.submit({
        draft: ref ? { ...draft, mask_id: ref.mask_id } : draft,
        estimate_id: est.estimate_id,
        confirmations: plan.confirmations,
      });
      set((cur) => ({
        submitting: false,
        confirmOpen: false,
        selectedJobId: job_id,
        selectedCandidateId: null,
        preview: null,
        live: {
          ...cur.live,
          [job_id]: cur.live[job_id] ?? {
            state: 'submitted',
            progress: 0,
            cost: est.cost,
            error: null,
            longRunning: false,
            startedAt: Date.now(),
          },
        },
      }));
      editorBridge.setCompare(null);
      settleReferences(); // the Retry flags are history now (FR-REF-26)
      await Promise.all([refreshIterations(), loadSpend(), refreshProject()]);
    } catch (e) {
      const err = uiError(e);
      if (err.code === 'over_spend_cap' || err.code === 'confirmation_missing') {
        await loadSpend();
        set({ submitting: false, confirmError: err });
        return;
      }
      set({ submitting: false, confirmOpen: false, submitError: err });
      if (err.code === 'estimate_stale') debouncer.force();
      // FR-REF-21: a ticked reference is gone; the block shows which, and the next edit or
      // resolve re-estimates.
      if (err.code === 'reference_unavailable') refreshReferences();
      void refreshIterations();
    }
  },

  async cancelJob(jobId: string): Promise<void> {
    try {
      const { result } = await api.region.cancel(jobId);
      set({
        trackerNotice:
          result === 'canceled'
            ? { kind: 'info', text: 'Job canceled.' }
            : { kind: 'info', text: 'This job can’t be canceled at this point; it continues.' },
      });
    } catch (e) {
      const err = uiError(e);
      set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
    }
    void refreshIterations();
  },

  /** FR-RGN-21: pre-fill the panel from the job, then estimate → confirm again. */
  async retry(jobId: string): Promise<void> {
    try {
      const d = await api.region.retryDraft(jobId);
      set({
        prompt: d.final_prompt,
        seedMode: d.seed === null ? 'random' : 'pinned',
        seed: d.seed,
        margin: d.region.margin,
        inner: d.region.inner,
        maskId: d.mask_id,
        submitError: null,
        trackerNotice: {
          kind: 'info',
          text: 'The panel is filled from the job. Check it, then submit.',
        },
      });
      debouncer.reset();
      // FR-RGN-44: the job's references, with changed / missing flags (REF resets its block).
      restoreReferences(d.references ?? null);
      await selectModel(d.model_id, d.controls);
      saveDraftSoon();
    } catch (e) {
      const err = uiError(e);
      set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
    }
  },

  async dismiss(jobId: string): Promise<void> {
    try {
      const { trashed } = await api.region.dismiss(jobId);
      set({
        trackerNotice: {
          kind: 'info',
          text: trashed
            ? `Dismissed; ${trashed} candidate${trashed === 1 ? '' : 's'} moved to the trash.`
            : 'Dismissed.',
        },
      });
    } catch (e) {
      const err = uiError(e);
      set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
    }
    await Promise.all([refreshIterations(), refreshProject()]);
  },

  selectIteration(jobId: string): void {
    if (state.selectedJobId === jobId) return;
    set({ selectedJobId: jobId, selectedCandidateId: null, preview: null, trackerNotice: null });
    editorBridge.setCompare(null);
    syncPicker();
  },

  async selectCandidate(id: string): Promise<void> {
    set({ selectedCandidateId: id, preview: null });
    await previewSelected();
    syncPicker();
  },

  setBlend(patch: Partial<RgnBlend>): void {
    set((s) => ({ blend: { ...s.blend, ...patch } }));
    if (previewTimer) clearTimeout(previewTimer);
    previewTimer = setTimeout(() => {
      previewTimer = null;
      void previewSelected();
    }, PREVIEW_DEBOUNCE_MS);
  },

  async accept(candidateId: string): Promise<void> {
    const s = state;
    const jobId = s.selectedJobId;
    if (!jobId) return;
    const parent = s.headRevision;
    try {
      const r = await api.region.accept({
        job_id: jobId,
        candidate_id: candidateId,
        blend: s.blend,
      });
      if (parent) pushUndo(revisionUndoEntry('Accept patch', parent, r.revision_id));
      set({ prompt: '', trackerNotice: { kind: 'info', text: 'Patch accepted.' }, preview: null });
      editorBridge.setCompare(null);
      saveDraftSoon();
    } catch (e) {
      const err = uiError(e);
      set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
    }
    await Promise.all([refreshProject(), refreshIterations()]);
  },

  /** FR-RGN-29: re-bake the accepted patch of the selected candidate with the current blend. */
  async retune(): Promise<void> {
    const s = state;
    const layer = s.selectedCandidateId ? layerOf(s, s.selectedCandidateId) : undefined;
    if (!layer) return;
    const parent = s.headRevision;
    try {
      const r = await api.region.retune(layer.id, s.blend);
      if (parent) pushUndo(revisionUndoEntry('Re-tune patch', parent, r.revision_id));
      set({ trackerNotice: { kind: 'info', text: 'Patch re-tuned.' } });
    } catch (e) {
      const err = uiError(e);
      set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
    }
    await refreshProject();
  },

  async star(candidateId: string, starred: boolean): Promise<void> {
    const jobId = state.selectedJobId;
    if (!jobId) return;
    try {
      await api.region.star(jobId, candidateId, starred);
    } catch (e) {
      const err = uiError(e);
      set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
    }
    await refreshProject();
  },

  async discard(candidateId: string): Promise<void> {
    const jobId = state.selectedJobId;
    if (!jobId) return;
    try {
      await api.region.discard(jobId, candidateId);
      if (state.selectedCandidateId === candidateId) {
        set({ selectedCandidateId: null, preview: null });
        editorBridge.setCompare(null);
      }
    } catch (e) {
      const err = uiError(e);
      set({ trackerNotice: { kind: 'error', text: errorText(err.code, err.message) } });
    }
    await Promise.all([refreshProject(), refreshIterations()]);
  },

  clearTrackerNotice(): void {
    set({ trackerNotice: null });
  },
};

export const selectedLayer = (s: RgnState) =>
  s.selectedCandidateId ? layerOf(s, s.selectedCandidateId) : undefined;

export function useRgn<T>(select: (s: RgnState) => T): T {
  return useSyncExternalStore(
    rgn.subscribe,
    () => select(state),
    () => select(state),
  );
}
