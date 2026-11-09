// The only module that talks to Tauri (AGENTS §9). Features call these typed wrappers; an
// ESLint rule forbids `@tauri-apps/*` imports anywhere else. Playwright mocks the layer below
// this one (window.__TAURI_INTERNALS__), so the wrapper itself is under test.
import { invoke } from '@tauri-apps/api/core';
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open as openDialog } from '@tauri-apps/plugin-dialog';

import type {
  ChatHostId,
  CommandName,
  Commands,
  CoreErrorShape,
  DraftReferences,
  Events,
  GenJobDraft,
  GenSize,
  Operation,
  ProjectKind,
  ExportSettingsInput,
  MaskBytes,
  ProjectFolder,
  PromptHelperInput,
  RectPx,
  RefRole,
  Section,
} from './types';

export type * from './types';

/** Sentence case for text shown as an error: core messages (and text built from codes) start
 *  lower case, Rust-style (BL-27). Only a plain leading word is changed, so a message that
 *  starts with a file name or identifier (`pack.json …`, `model_id …`) keeps it as written. */
export function sentence(text: string): string {
  return /^[a-z][a-z']*(?=[\s,:;!?)]|$)/.test(text) ? text[0]!.toUpperCase() + text.slice(1) : text;
}

/** A failed core command, with its normalised code (`read_only`, `not_found`, …). Its
 *  `message` is in sentence case, ready to show. */
export class ApiError extends Error {
  readonly code: string;
  readonly field: string | undefined;
  readonly detail: unknown;

  constructor(shape: CoreErrorShape) {
    super(sentence(shape.message));
    this.name = 'ApiError';
    this.code = shape.code;
    this.field = shape.field;
    this.detail = shape.detail;
  }
}

function toApiError(e: unknown): ApiError {
  if (e instanceof ApiError) return e;
  if (e && typeof e === 'object' && 'code' in e && 'message' in e) {
    return new ApiError(e as CoreErrorShape);
  }
  // Tauri rejects with a string when the command itself can't be called (e.g. bad args).
  return new ApiError({ code: 'ipc', message: String(e) });
}

/** Runs a core command. Every command takes one `input` object. */
export async function call<N extends CommandName>(
  name: N,
  input: Commands[N][0],
): Promise<Commands[N][1]> {
  try {
    return await invoke<Commands[N][1]>(name, { input });
  } catch (e) {
    throw toApiError(e);
  }
}

/** True inside the Tauri window (or a test that mocks it). */
export function hasCore(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The label of the window this page runs in: `main` or `prompt-chat` (R91). Tauri gives it in
 *  its metadata; a plain browser page (Playwright) may pass `?window=<label>`. */
export function currentWindowLabel(): string {
  const meta = (
    window as unknown as {
      __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
    }
  ).__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  return meta ?? new URLSearchParams(window.location.search).get('window') ?? 'main';
}

/** Sends an event to every window (BL-42: the chat window asks the main window to flush). */
export function emitEvent<E extends keyof Events>(name: E, payload: Events[E]): Promise<void> {
  return emit(name, payload);
}

/** Subscribes to a core event. Returns the unsubscribe function. */
export function onEvent<E extends keyof Events>(
  name: E,
  handler: (payload: Events[E]) => void,
): Promise<UnlistenFn> {
  return listen<Events[E]>(name, (event) => handler(event.payload));
}

export type DropEvent = { type: 'over' } | { type: 'leave' } | { type: 'drop'; paths: string[] };

/** Native file drag-and-drop on the window (paths, not File objects). */
export function onFileDrop(handler: (event: DropEvent) => void): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    if (p.type === 'drop') handler({ type: 'drop', paths: p.paths });
    else if (p.type === 'leave') handler({ type: 'leave' });
    else handler({ type: 'over' });
  });
}

/** Images the app accepts (FR-PRJ-06), for the open dialog's filter. */
export const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'webp'];

export interface PickOptions {
  title: string;
  /** Pick folders instead of files. */
  directory?: boolean;
  multiple?: boolean;
  /** Only images (FR-PRJ-06). */
  images?: boolean;
}

/** The native open dialog (#53). Resolves to the picked paths ([] when canceled). Rejects with
 *  an `ApiError` when no dialog is available (Vitest, Playwright): callers fall back to typed
 *  paths. */
export async function pickPaths(opts: PickOptions): Promise<string[]> {
  try {
    const picked = await openDialog({
      title: opts.title,
      directory: opts.directory ?? false,
      multiple: opts.multiple ?? false,
      ...(opts.images ? { filters: [{ name: 'Images', extensions: IMAGE_EXTENSIONS }] } : {}),
    });
    if (picked === null) return [];
    return Array.isArray(picked) ? picked : [picked];
  } catch (e) {
    throw toApiError(e);
  }
}

/** Tells the app the UI has rendered (ends the process in `tauri:smoke` runs). In a debug smoke
 *  run with a tile step the app answers with the image to open next (R46); otherwise null. */
export async function appReady(ok: boolean, detail: string): Promise<string | null> {
  try {
    return (await invoke<string | null>('app_ready', { ok, detail })) ?? null;
  } catch {
    // Not fatal: only the smoke run depends on it.
    return null;
  }
}

/** R101: tells the app the close guards listen for `app:close-requested`, so the window X waits
 *  for them. Not fatal when it fails (no Tauri, Playwright): X then closes as before. */
export async function appCloseGuard(): Promise<void> {
  try {
    await invoke('app_close_guard');
  } catch {
    // Nothing to guard against outside the app.
  }
}

/** R109: the close guards stopped listening (the UI unmounted, a crash): the window X no longer
 *  waits for them. */
export async function appCloseUnguard(): Promise<void> {
  try {
    await invoke('app_close_unguard');
  } catch {
    // Nothing was guarded outside the app.
  }
}

/** R109: the UI got `app:close-requested`; one not acked within 3 s closes the app anyway.
 *  True if the app really asked (only Rust sends it); false: ignore the event. */
export async function appCloseAck(): Promise<boolean> {
  try {
    return (await invoke<boolean>('app_close_ack')) === true;
  } catch {
    // Outside the app there is no close to ack.
    return false;
  }
}

/** R101: the project is closed (or none was open): the app exits. */
export async function appExit(): Promise<void> {
  try {
    await invoke('app_exit');
  } catch {
    // Outside the app there is nothing to exit.
  }
}

/** Standard base64 of binary data (masks over IPC; ~4× smaller than a JSON number array). */
export function toBase64(bytes: Uint8Array): string {
  let bin = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK)
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK) as unknown as number[]);
  return btoa(bin);
}

export function fromBase64(text: string): Uint8Array {
  const bin = atob(text);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Typed shortcuts for the commands, grouped by area. */
export const api = {
  appInfo: () => call('app_info', {}),
  library: {
    getConfig: () => call('library_get_config', {}),
    setRoots: (roots: string[]) => call('library_set_roots', { roots }),
    listProjects: (section: Section) => call('library_list_projects', { section }),
  },
  project: {
    open: (path: string) => call('project_open', { path }),
    close: (force = false) => call('project_close', force ? { force: true } : {}),
    get: () => call('project_get', {}),
    putDraft: (draft: Commands['project_put_draft'][0]['draft']) =>
      call('project_put_draft', { draft }),
    saveRevision: (label?: string) =>
      call('project_save_revision', label === undefined ? {} : { label }),
    listRevisions: () => call('project_list_revisions', {}),
    restoreRevision: (revId: string) => call('project_restore_revision', { rev_id: revId }),
    create: (section: Section, paths: string[]) => call('project_create', { section, paths }),
    createCancel: () => call('project_create_cancel', {}),
    createFromClipboard: (section: Section, dataB64: string) =>
      call('project_create_from_clipboard', { section, data_b64: dataB64 }),
    rename: (path: string, name: string) => call('project_rename', { path, name }),
    folderList: (folder: ProjectFolder) => call('project_folder_list', { folder }),
    folderAdd: (folder: ProjectFolder, paths: string[]) =>
      call('project_folder_add', { folder, paths }),
    trashList: () => call('project_trash_list', {}),
    trashRestore: (itemId: string) => call('project_trash_restore', { item_id: itemId }),
    diskUsage: () => call('project_disk_usage', {}),
    relinkOriginal: (path: string) => call('project_relink_original', { path }),
    conflictGet: () => call('project_conflict_get', {}),
    resolveConflict: (keep: 'this' | 'other') => call('project_resolve_conflict', { keep }),
  },
  editor: {
    imageOpen: (projectId: string, imageId: string, maxSide?: number) =>
      call('edt_image_open', {
        project_id: projectId,
        image_id: imageId,
        ...(maxSide === undefined ? {} : { max_side: maxSide }),
      }),
    tilesOpen: (projectId: string, imageId: string, rebuild = false) =>
      call('edt_tiles_open', {
        project_id: projectId,
        image_id: imageId,
        ...(rebuild ? { rebuild: true } : {}),
      }),
  },
  export: {
    getSettings: () => call('export_get_settings', {}),
    setSettings: (settings: ExportSettingsInput) => call('export_set_settings', { settings }),
    plan: (settings: ExportSettingsInput) => call('export_plan', { settings }),
    start: (settings: ExportSettingsInput, replace: boolean) =>
      call('export_start', { settings, replace }),
    cancel: (exportId: string) => call('export_cancel', { export_id: exportId }),
    /** PRJ's command (FR-PRJ-42); EXP calls it when its panel opens (FR-EXP-28). */
    prefetchOriginal: () => call('project_prefetch_original', {}),
  },
  region: {
    fit: (
      projectId: string,
      inner: Commands['rgn_region_fit'][0]['inner'],
      margin?: number,
      modelId?: string,
    ) =>
      call('rgn_region_fit', {
        project_id: projectId,
        inner,
        ...(margin === undefined ? {} : { margin }),
        ...(modelId === undefined ? {} : { model_id: modelId }),
      }),
    evaluateControls: (modelId: string, values: Record<string, unknown>) =>
      call('rgn_controls_evaluate', { model_id: modelId, values }),
    estimate: (projectId: string, draft: Commands['rgn_job_estimate'][0]['draft']) =>
      call('rgn_job_estimate', { project_id: projectId, draft }),
    getSpend: () => call('rgn_spend_get', {}),
    setSpend: (spend: Commands['rgn_spend_set'][0]) => call('rgn_spend_set', spend),
    submit: (input: Commands['rgn_job_submit'][0]) => call('rgn_job_submit', input),
    cancel: (jobId: string) => call('rgn_job_cancel', { job_id: jobId }),
    retryDraft: (jobId: string) => call('rgn_job_retry_draft', { job_id: jobId }),
    dismiss: (jobId: string) => call('rgn_job_dismiss', { job_id: jobId }),
    fetchAgain: (jobId: string) => call('rgn_job_fetch_again', { job_id: jobId }),
    preview: (input: Commands['rgn_candidate_preview'][0]) => call('rgn_candidate_preview', input),
    accept: (input: Commands['rgn_candidate_accept'][0]) => call('rgn_candidate_accept', input),
    star: (jobId: string, candidateId: string, starred: boolean) =>
      call('rgn_candidate_star', { job_id: jobId, candidate_id: candidateId, starred }),
    discard: (jobId: string, candidateId: string) =>
      call('rgn_candidate_discard', { job_id: jobId, candidate_id: candidateId }),
    /** #56: "Discard the rest" of a job (one commit, one trash item). */
    discardRest: (jobId: string) => call('rgn_job_discard_rest', { job_id: jobId }),
    retune: (layerId: string, blend: Commands['rgn_patch_retune'][0]['blend']) =>
      call('rgn_patch_retune', { layer_id: layerId, blend }),
    iterations: () => call('rgn_iterations_list', {}),
    getBlendInstruction: () => call('rgn_blend_instruction_get', {}),
    setBlendInstruction: (text: string | null) => call('rgn_blend_instruction_set', { text }),
  },
  mask: {
    /** FR-EDT-18: the mask's w·h bytes for `rect` → `MaskRef` (base64 over IPC). */
    submit: (projectId: string, rect: RectPx, bytes: Uint8Array) =>
      call('edt_mask_submit', { project_id: projectId, rect, data_b64: toBase64(bytes) }),
    read: async (projectId: string, maskId: string): Promise<MaskBytes> => {
      const r = await call('edt_mask_read', { project_id: projectId, mask_id: maskId });
      return { rect: r.rect, bytes: fromBase64(r.data_b64) };
    },
  },
  provider: {
    status: () => call('provider_status', {}),
    setCredentials: (key: string, secret: string) =>
      call('provider_credentials_set', { key, secret }),
    clearCredentials: () => call('provider_credentials_clear', {}),
    testConnection: () => call('provider_test_connection', {}),
    getSettings: () => call('provider_settings_get', {}),
    setSettings: (teamId: string | null, projectId: string | null) =>
      call('provider_settings_set', { team_id: teamId, project_id: projectId }),
    listModels: (operation: Commands['provider_list_models'][0]['operation']) =>
      call('provider_list_models', { operation }),
    getCapabilities: (modelId: string, operation?: Operation) =>
      call('provider_get_capabilities', {
        model_id: modelId,
        ...(operation === undefined ? {} : { operation }),
      }),
    balance: () => call('provider_balance', {}),
    fakeSetCase: (fakeCase: string) => call('provider_fake_set_case', { case: fakeCase }),
  },
  references: {
    listPacks: () => call('ref_list_packs', {}),
    getPack: (packId: string) => call('ref_pack_get', { pack_id: packId }),
    create: (name: string, paths?: string[], suffixIfTaken = false) =>
      call('ref_pack_create', {
        name,
        ...(paths?.length ? { paths } : {}),
        ...(suffixIfTaken ? { suffix_if_taken: true } : {}),
      }),
    rename: (packId: string, name: string) => call('ref_pack_rename', { pack_id: packId, name }),
    update: (packId: string, patch: { notes?: string; roles?: Record<string, RefRole | null> }) =>
      call('ref_pack_update', { pack_id: packId, ...patch }),
    addImages: (packId: string, paths: string[]) =>
      call('ref_pack_add_images', { pack_id: packId, paths }),
    removeImages: (packId: string, files: string[]) =>
      call('ref_pack_remove_images', { pack_id: packId, files }),
    delete: (packId: string) => call('ref_pack_delete', { pack_id: packId }),
    trashList: () => call('ref_trash_list', {}),
    trashRestore: (itemId: string) => call('ref_trash_restore', { id: itemId }),
    /** GEN passes `{operation: 'generate', structure}` (FR-GEN-13); RGN passes nothing. */
    resolve: (
      modelId: string,
      references: DraftReferences | null,
      gen?: { operation: Operation; structure: string | null },
    ) => call('ref_resolve', { model_id: modelId, references, ...(gen ?? {}) }),
  },
  /** new-asset-generation §9 (GEN). Cancel, dismiss, star, discard and the iterations list are
   *  RGN's commands, reused as is. */
  gen: {
    projectCreate: (section: Section, name: string) =>
      call('gen_project_create', { section, name }),
    sizeFit: (modelId: string, kind: ProjectKind, size: GenSize) =>
      call('gen_size_fit', { model_id: modelId, kind, size }),
    evaluateControls: (modelId: string, values: Record<string, unknown>) =>
      call('gen_controls_evaluate', { model_id: modelId, values }),
    estimate: (draft: GenJobDraft) => call('gen_job_estimate', { draft }),
    submit: (input: Commands['gen_job_submit'][0]) => call('gen_job_submit', input),
    retryDraft: (jobId: string) => call('gen_job_retry_draft', { job_id: jobId }),
    preview: (jobId: string, candidateId: string) =>
      call('gen_candidate_preview', { job_id: jobId, candidate_id: candidateId }),
    pick: (jobId: string, candidateId: string) =>
      call('gen_candidate_pick', { job_id: jobId, candidate_id: candidateId }),
  },
  /** prompt-helper §9 (PRM): helper definitions (Settings) and the final prompt preview. */
  prompt: {
    listHelpers: (kind?: ProjectKind) =>
      call('prm_helpers_list', kind === undefined ? {} : { kind }),
    saveHelpers: (helpers: PromptHelperInput[]) => call('prm_helpers_save', { helpers }),
    resetHelper: (id: string) => call('prm_helpers_reset', { id }),
    resetAllHelpers: () => call('prm_helpers_reset', { all: true }),
    composePreview: (input: Commands['prm_compose_preview'][0]) =>
      call('prm_compose_preview', input),
  },
  /** prompt-helper §9 (PRM part B): the prompt chat core (slice 5). The window is slice 6. */
  chat: {
    status: (refresh = false) => call('prm_chat_status', refresh ? { refresh } : {}),
    consent: (accept: boolean) => call('prm_chat_consent', { accept }),
    settingsPut: (input: Commands['prm_chat_settings_put'][0]) =>
      call('prm_chat_settings_put', input),
    logout: () => call('prm_chat_logout', {}),
    sessionEnd: () => call('prm_chat_session_end', {}),
    contextPublish: (projectId: string, host: ChatHostId, input: unknown) =>
      call('prm_context_publish', { project_id: projectId, host, input }),
    contextClear: (projectId: string, host: ChatHostId) =>
      call('prm_context_clear', { project_id: projectId, host }),
    thread: (projectId: string) => call('prm_chat_thread_get', { project_id: projectId }),
    send: (projectId: string, text: string, includeImages = true) =>
      call('prm_chat_send', { project_id: projectId, text, include_images: includeImages }),
    cancel: (turnId: string) => call('prm_chat_cancel', { turn_id: turnId }),
    insert: (projectId: string, proposalId: string) =>
      call('prm_chat_insert', { project_id: projectId, proposal_id: proposalId }),
    fakeSetCase: (c: string) => call('prm_chat_fake_set_case', { case: c }),
    /** What the next turn would send, for the context strip (FR-PRM-26, BL-42). */
    preview: (projectId: string, includeImages = true) =>
      call('prm_chat_preview', { project_id: projectId, include_images: includeImages }),
    /** Opens or focuses the chat window (FR-PRM-21). */
    openWindow: () => call('prm_window_open', {}),
    /** Opens the login console (FR-PRM-45). */
    login: () => call('prm_chat_login', {}),
  },
};
