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
  overridden: boolean;
  configured: boolean;
}

/** A library section (#44): the rail entry and its folder (`Maps/`, `Tokens/`, `Items/`). */
export type Section = 'maps' | 'tokens' | 'items';

/** `project.json` `kind`, from the section (FR-PRJ-46). */
export type ProjectKind = 'map' | 'token' | 'item';

export type SkipReason = 'unsupported' | 'animated' | 'too_large' | 'corrupt' | 'not_found' | 'io';

/** One source of `project_create` (FR-PRJ-03..08). `path` is the project folder. */
export type CreateItem =
  | { status: 'created'; source: string; path: string }
  | { status: 'already_in_library'; source: string; path: string }
  | { status: 'skipped'; source: string; reason: SkipReason };

export type CardBadge = 'read_only' | 'conflict' | 'unreadable' | 'not_available_offline';

/** A home-grid card (FR-PRJ-49), read from `project.json` and `.pforge/thumb.jpg` only. */
export interface ProjectCard {
  /** `project_id`; the folder path when `project.json` is unreadable. */
  id: string;
  /** The project folder (absolute), the argument of `project_open` / `project_rename`. */
  path: string;
  name: string;
  kind: ProjectKind;
  /** Jobs with an iteration number (manifest `jobs[].iteration`). */
  iterations: number;
  /** `commit.at`, ISO time. */
  last_edited: string;
  /** Thumbnail URL on the `pf-tiles` scheme (EDT `library` key space, OQ-PRJ-16), or null. */
  thumb_url: string | null;
  badges: CardBadge[];
}

/** Project folders `project_folder_list` / `project_folder_add` take (FR-PRJ-47). */
export type ProjectFolder = 'working' | 'references' | 'structure' | 'parts' | 'exports';

/** A file in a project folder (FR-PRJ-48). `image` = a supported image (it gets a thumbnail). */
export interface FolderFile {
  name: string;
  bytes: number;
  mtime: string;
  image: boolean;
  /** Thumbnail URL on the `pf-tiles` scheme for images, or null (hydrated only on view). */
  thumb_url?: string | null;
}

export interface TrashItem {
  id: string;
  /** Project-relative path the item came from. */
  path: string;
  kind: string;
  bytes: number;
  trashed_at: string;
}

export type DiskCategory =
  | 'original'
  | 'layers'
  | 'masks'
  | 'job_inputs'
  | 'results'
  | 'trash'
  | 'user_folders'
  | 'exports'
  | 'other';

/** `project_disk_usage` (FR-PRJ-33): bytes per category, from file metadata only. */
export interface DiskUsage {
  bytes: Record<DiskCategory, number>;
  trash_count: number;
  /** When the oldest trash item is purged (30 days, FR-PRJ-32), or null with an empty trash. */
  next_purge: string | null;
}

/** RGN's iteration list (region-edit §9), shown in the left panel's Results. */
export interface IterationEntry {
  job_id: string;
  n: number;
  dir: string;
  slug: string;
  state: string;
  kept: number;
  returned: number;
  cost: number | null;
  accepted_in_stack: boolean;
}

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

/** `project_open` warning codes (asset-project §9): read-only reasons, `conflict`, `recovered`,
 *  `resumable`, `lost`, `original_missing` (Relink). */

export interface ProjectWarning {
  code: string;
  message: string;
}

export interface ProjectSnapshot {
  project_id: string;
  kind: ProjectKind;
  /** The project folder name (FR-PRJ-44). */
  name: string;
  /** The original, `<project_dir>/working/<file>`. */
  image_path: string;
  /** The project folder (absolute). */
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
  /** `commit.at` of the loaded manifest (the status bar's "Saved · time"). */
  committed_at?: string;
}

export interface DraftInput {
  final_prompt?: string;
  model_id?: string | null;
  controls?: Record<string, unknown>;
  seed?: number | null;
  region?: unknown;
  mask_id?: string | null;
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
  library_set_roots: [{ roots: string[] }, LibraryConfig];
  project_open: [{ path: string }, ProjectSnapshot];
  project_close: [Record<string, never>, Record<string, never>];
  project_get: [Record<string, never>, ProjectSnapshot | null];
  project_put_draft: [{ draft: DraftInput }, { seq: number }];
  project_save_revision: [{ label?: string }, RevisionEntry];
  project_list_revisions: [Record<string, never>, RevisionEntry[]];
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
  // asset-project §9 (#44)
  library_list_projects: [{ section: Section }, ProjectCard[]];
  project_create: [{ section: Section; paths: string[] }, { items: CreateItem[] }];
  project_create_cancel: [Record<string, never>, Record<string, never>];
  project_create_from_clipboard: [{ section: Section; data_b64: string }, { path: string }];
  project_rename: [{ path: string; name: string }, { path: string }];
  project_folder_list: [{ folder: ProjectFolder }, FolderFile[]];
  project_folder_add: [{ folder: ProjectFolder; paths: string[] }, FolderFile[]];
  project_trash_list: [Record<string, never>, TrashItem[]];
  project_trash_restore: [{ item_id: string }, Record<string, never>];
  project_disk_usage: [Record<string, never>, DiskUsage];
  project_relink_original: [{ path: string }, Record<string, never>];
  rgn_iterations_list: [Record<string, never>, IterationEntry[]];
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
  'import:progress': { done: number; total: number };
  'project:save-failed': { reason: string; retrying: boolean };
  'project:conflict': Record<string, never>;
  'project:recovered': Record<string, never>;
  'project:thumb-updated': { project_id: string };
}
