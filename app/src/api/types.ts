// Shapes of core command inputs and outputs (pf-core `commands.rs`). Keep in step with Rust.

/** Normalised command error (`CoreError`). */
export interface CoreErrorShape {
  code: string;
  message: string;
  field?: string;
  detail?: unknown;
}

export interface SettingsWarning {
  code: string;
  message: string;
}

export interface AppInfo {
  version: string;
  provider_mode: 'fake' | 'scenario';
  data_dir: string;
  library_overridden: boolean;
  warnings: SettingsWarning[];
}

export interface RootStatus {
  path: string;
  available: boolean;
}

export interface LibraryConfig {
  roots: RootStatus[];
  import_dest: string | null;
  overridden: boolean;
  configured: boolean;
}

export type SkipReason = 'unsupported' | 'animated' | 'too_large' | 'corrupt' | 'not_found' | 'io';

export type ImportItem =
  | { status: 'imported'; source: string; path: string }
  | { status: 'already_in_library'; source: string; path: string }
  | { status: 'skipped'; source: string; reason: SkipReason };

export interface Layer {
  id: string;
  kind: 'original' | 'patch';
  file: string | null;
  cx: number;
  cy: number;
  scale: number;
  rotation: number;
  opacity: number;
  visible: boolean;
}

export interface Draft {
  final_prompt: string;
  model_id: string | null;
  controls: Record<string, unknown>;
  seed: number | null;
  region: unknown;
  mask_id: string | null;
  updated_at: string;
}

export type RevisionReason = 'create' | 'accept' | 'save' | 'restore' | 'retune' | 'conflict';

export interface RevisionEntry {
  id: string;
  seq: number;
  created_at: string;
  reason: RevisionReason;
  label?: string;
}

export type ReadOnlyReason = 'newer_version' | 'original_changed' | 'original_missing';

export interface ProjectWarning {
  code: string;
  message: string;
}

export interface ProjectSnapshot {
  project_id: string;
  name: string;
  image_path: string;
  project_dir: string;
  saved: boolean;
  read_only: ReadOnlyReason | null;
  warnings: ProjectWarning[];
  width: number;
  height: number;
  format: 'png' | 'jpeg' | 'webp';
  layers: Layer[];
  draft: Draft | null;
  revisions: RevisionEntry[];
  head_revision: string | null;
  commit_seq: number;
}

export interface DraftInput {
  final_prompt?: string;
  model_id?: string | null;
  controls?: Record<string, unknown>;
  seed?: number | null;
  region?: unknown;
  mask_id?: string | null;
}

export interface RecentEntry {
  path: string;
  name: string;
  opened_at: string;
}

export interface ImageOpen {
  key: string;
  url: string;
  width: number;
  height: number;
}

export type ProviderMode = 'fake' | 'scenario';

export type Connectivity = 'unknown' | 'ok' | 'offline' | 'auth_error';

export interface ProviderStatus {
  mode: ProviderMode;
  configured: boolean;
  /** Last 4 characters of the stored key id; the secret is never returned (FR-PRV-07). */
  key_suffix?: string;
  connectivity: Connectivity;
  store_unavailable: boolean;
}

export interface ProviderSettings {
  team_id: string | null;
  project_id: string | null;
}

export type Operation =
  'generate' | 'edit_masked' | 'extend' | 'upscale' | 'remove_background' | 'tile';

export interface ModelSummary {
  id: string;
  name: string;
  operations: Operation[];
}

export type Balance =
  | { kind: 'known'; remaining: number; unit: string; period_end: string | null }
  | { kind: 'unknown' };

/** Generic model capabilities (provider spec Appendix C). */
export interface Capabilities {
  model_id: string;
  operations: Operation[];
  controls: {
    name: string;
    type: 'int' | 'float' | 'enum' | 'bool' | 'string' | 'seed' | 'image' | 'mask';
    role: 'source' | 'mask' | 'prompt' | 'candidate_count' | 'seed' | 'reference' | 'other';
    min: number | null;
    max: number | null;
    step: number | null;
    allowed: unknown[];
    default: unknown;
    required: boolean;
    cost_affecting: boolean;
  }[];
  rules: {
    if: { control: string; op: 'eq' | 'in'; value: unknown };
    then: { control: string; required?: boolean; allowed?: unknown[]; hidden?: boolean };
  }[];
  mask: {
    convention: 'white_edit' | 'black_edit' | 'alpha_transparent_edit';
    soft: { rule: 'pass_through' } | { rule: 'threshold'; level: number };
  } | null;
  input_px: { min: number; max: number; multiple_of: number };
  max_bytes: number;
  seed: boolean;
  prompt_required: boolean;
  slots: { name: string; role: string | null; required: boolean }[];
  warnings: string[];
}

/** Core command name → [input, output]. Append new commands. */
export interface Commands {
  app_info: [Record<string, never>, AppInfo];
  library_get_config: [Record<string, never>, LibraryConfig];
  library_set_roots: [{ roots: string[]; import_dest?: string | null }, LibraryConfig];
  import_paths: [{ paths: string[] }, { items: ImportItem[] }];
  project_open: [{ path: string }, ProjectSnapshot];
  project_close: [Record<string, never>, Record<string, never>];
  project_get: [Record<string, never>, ProjectSnapshot | null];
  project_put_draft: [{ draft: DraftInput }, { seq: number }];
  project_save_revision: [{ label?: string }, RevisionEntry];
  project_list_revisions: [Record<string, never>, RevisionEntry[]];
  recent_list: [Record<string, never>, RecentEntry[]];
  edt_image_open: [{ project_id: string; image_id: string }, ImageOpen];
  provider_status: [Record<string, never>, ProviderStatus];
  provider_credentials_set: [{ key: string; secret: string }, Record<string, never>];
  provider_credentials_clear: [Record<string, never>, Record<string, never>];
  provider_test_connection: [Record<string, never>, Record<string, never>];
  provider_settings_get: [Record<string, never>, ProviderSettings];
  provider_settings_set: [
    { team_id?: string | null; project_id?: string | null },
    ProviderSettings,
  ];
  provider_list_models: [{ operation: Operation }, { models: ModelSummary[]; stale: boolean }];
  provider_get_capabilities: [{ model_id: string }, Capabilities];
  provider_balance: [Record<string, never>, Balance];
  provider_fake_set_case: [{ case: string }, Record<string, never>];
}

export type CommandName = keyof Commands;

/** Core events (name → payload). Append new events. */
export interface Events {
  'project:committed': { seq: number };
  'import:done': { total: number };
  'provider:status-changed': { connectivity: Connectivity };
}
