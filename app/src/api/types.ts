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

/** One pyramid level (EDT Appendix B). `scale` = 1 / 2^level. */
export interface TilesLevel {
  level: number;
  scale: number;
  cols: number;
  rows: number;
  ext: 'jpg' | 'webp';
  complete: boolean;
}

/** `edt_tiles_open` result. Tiles are at `${base_url}/<level>/<col>_<row>.<ext>`. */
export interface TilesDescriptor {
  key: string;
  base_url: string;
  width: number;
  height: number;
  tile_size: number;
  levels: TilesLevel[];
}

/** EXP's settings as EDT reads them (FR-EXP-26); the rest of the shape is EXP's. */
export interface ExportSettings {
  source_px_per_square: number | null;
  target_px_per_square: number | null;
  [k: string]: unknown;
}

/** Integer image-px rectangle. */
export interface RectPx {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** `rgn_region_fit` result (RGN §9). */
export interface RegionFit {
  inner: RectPx;
  outer: RectPx;
  margin: number;
  sent_w: number;
  sent_h: number;
  scale: number;
}

/** One control as the Generate panel renders it (`rgn_controls_evaluate`, FR-RGN-33). */
export interface ControlState {
  name: string;
  type: Capabilities['controls'][number]['type'];
  role: Capabilities['controls'][number]['role'];
  min: number | null;
  max: number | null;
  step: number | null;
  default: unknown;
  cost_affecting: boolean;
  /** False when a rule hides it: neither shown nor sent. */
  visible: boolean;
  /** Effective allowed values (empty = any value in range). */
  allowed: unknown[];
  required: boolean;
  /** The value to show: the user's, or the default after a reset; null = unset. */
  value: unknown;
  /** The user's value was rejected and reset; show a notice. */
  reset: boolean;
  error?: { control: string; code: string; message: string };
}

/** `rgn_controls_evaluate` result. Image inputs (source, mask, reference) are never listed. */
export interface ControlsEvaluation {
  model_id: string;
  controls: ControlState[];
  /** Required controls without a value: submit is disabled, naming them. */
  required_missing: string[];
  /** Given names this model has no control for (dropped with a notice, FR-RGN-11). */
  unknown: string[];
  warnings: string[];
  prompt_required: boolean;
  seed: boolean;
}

/** The job draft estimate and submit act on (RGN §9). */
export interface JobDraft {
  region: { inner: RectPx; margin?: number };
  mask_id: string;
  model_id: string;
  final_prompt: string;
  /** Control name → value, as the user set them (FR-RGN-34). */
  controls: Record<string, unknown>;
  /** Pinned seed, or null = random. */
  seed: number | null;
}

export type SpendNeed = 'threshold' | 'unknown_cost' | 'set_cap';

/** `rgn_job_estimate` result (FR-RGN-12..16). */
export interface JobEstimate {
  estimate_id: string;
  cost: { amount: number | 'unknown'; unit: string };
  fit: RegionFit;
  candidate_count: number;
  spend: number;
  cap: number | null;
  threshold: number | null;
  needs: SpendNeed[];
  /** spend + estimate > cap: submit fails with `over_spend_cap` until the cap is raised. */
  over_cap: boolean;
}

/** `rgn_spend_get` / `rgn_spend_set` result. */
export interface Spend {
  spend: number;
  cap: number | null;
  threshold: number | null;
}

/** `edt_mask_submit` result (FR-EDT-18). */
export interface MaskRef {
  mask_id: string;
  rect: RectPx;
}

/** `edt_mask_read` result: raw w·h bytes (an array over IPC) + rect. */
export interface MaskRead {
  rect: RectPx;
  bytes: number[] | ArrayBuffer;
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
  edt_image_open: [{ project_id: string; image_id: string; max_side?: number }, ImageOpen];
  provider_status: [Record<string, never>, ProviderStatus];
  edt_tiles_open: [{ project_id: string; image_id: string; rebuild?: boolean }, TilesDescriptor];
  export_get_settings: [Record<string, never>, ExportSettings];
  rgn_region_fit: [
    { project_id: string; inner: RectPx; margin?: number; model_id?: string | null },
    RegionFit,
  ];
  edt_mask_read: [{ project_id: string; mask_id: string }, MaskRead];
  project_restore_revision: [{ rev_id: string }, RevisionEntry];
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
  rgn_controls_evaluate: [
    { model_id: string; values: Record<string, unknown> },
    ControlsEvaluation,
  ];
  rgn_job_estimate: [{ project_id?: string; draft: JobDraft }, JobEstimate];
  rgn_spend_get: [Record<string, never>, Spend];
  /** A missing field keeps its value; null clears it. */
  rgn_spend_set: [{ cap?: number | null; threshold?: number | null }, Spend];
}

export type CommandName = keyof Commands;

/** Core events (name → payload). Append new events. */
export interface Events {
  'project:committed': { seq: number };
  'import:done': { total: number };
  'edt:tiles-progress': { key: string; level: number };
  'edt:tiles-ready': { key: string };
  'edt:tiles-failed': { key: string; error: CoreErrorShape | string };
  'export:settings-changed': { source: number | null; target: number | null };
  'provider:status-changed': { connectivity: Connectivity };
  'rgn:spend': { spend: number; cap: number | null };
}
