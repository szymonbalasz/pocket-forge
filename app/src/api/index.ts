// The only module that talks to Tauri (AGENTS §9). Features call these typed wrappers; an
// ESLint rule forbids `@tauri-apps/*` imports anywhere else. Playwright mocks the layer below
// this one (window.__TAURI_INTERNALS__), so the wrapper itself is under test.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';

import type {
  CommandName,
  Commands,
  CoreErrorShape,
  Events,
  ExportSettingsInput,
} from './types';

export type * from './types';

/** A failed core command, with its normalised code (`read_only`, `not_found`, …). */
export class ApiError extends Error {
  readonly code: string;
  readonly field: string | undefined;
  readonly detail: unknown;

  constructor(shape: CoreErrorShape) {
    super(shape.message);
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

/** Tells the app the UI has rendered (ends the process in `tauri:smoke` runs). */
export async function appReady(ok: boolean, detail: string): Promise<void> {
  try {
    await invoke('app_ready', { ok, detail });
  } catch {
    // Not fatal: only the smoke run depends on it.
  }
}

/** Typed shortcuts for the commands, grouped by area. */
export const api = {
  appInfo: () => call('app_info', {}),
  library: {
    getConfig: () => call('library_get_config', {}),
    setRoots: (roots: string[], importDest?: string | null) =>
      call('library_set_roots', { roots, import_dest: importDest ?? null }),
    importPaths: (paths: string[]) => call('import_paths', { paths }),
  },
  project: {
    open: (path: string) => call('project_open', { path }),
    close: () => call('project_close', {}),
    get: () => call('project_get', {}),
    putDraft: (draft: Commands['project_put_draft'][0]['draft']) =>
      call('project_put_draft', { draft }),
    saveRevision: (label?: string) =>
      call('project_save_revision', label === undefined ? {} : { label }),
    listRevisions: () => call('project_list_revisions', {}),
    restoreRevision: (revId: string) => call('project_restore_revision', { rev_id: revId }),
  },
  recent: { list: () => call('recent_list', {}) },
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
    fit: (projectId: string, inner: Commands['rgn_region_fit'][0]['inner'], margin?: number) =>
      call('rgn_region_fit', {
        project_id: projectId,
        inner,
        ...(margin === undefined ? {} : { margin }),
      }),
  },
  mask: {
    read: (projectId: string, maskId: string) =>
      call('edt_mask_read', { project_id: projectId, mask_id: maskId }),
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
    getCapabilities: (modelId: string) => call('provider_get_capabilities', { model_id: modelId }),
    balance: () => call('provider_balance', {}),
    fakeSetCase: (fakeCase: string) => call('provider_fake_set_case', { case: fakeCase }),
  },
};
