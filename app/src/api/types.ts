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

export type SkipReason =
  | 'unsupported'
  | 'animated'
  | 'too_large'
  | 'corrupt'
  | 'not_found'
  | 'io'
  | 'not_available_offline';

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
  /** `commit.at`, ISO time; null when `project.json` is unreadable (R45). */
  last_edited: string | null;
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
  /** Project-relative path the item came from (the first of `files`). */
  path: string;
  kind: string;
  bytes: number;
  trashed_at: string;
  /** Every project path in the item (core, PRJ slice 5). */
  files?: string[];
  /** `trashed_at` + 30 days (FR-PRJ-32). */
  purge_at?: string;
  reason?: string;
  /** Restore is offered only when true: `candidates`, `job_inputs`, `conflict_copy` (R51). */
  restorable: boolean;
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
  /** Sum of `bytes` (core). */
  total?: number;
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
  /** REF's selection (schema §4.5, #51); null = REF's defaults (FR-REF-17). */
  references?: DraftReferences | null;
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

/** `project_resolve_conflict` `keep` (FR-PRJ-36). */
export type ConflictKeep = 'this' | 'other';

/** One version in the conflict view (FR-PRJ-36). */
export interface ConflictSide {
  side: ConflictKeep;
  /** `project.json`, the conflict copy's name, or null for this machine's state in memory. */
  file: string | null;
  readable: boolean;
  committed_at: string | null;
  seq: number | null;
  machine: 'this' | 'other';
  layers: Layer[];
  newest_revision: RevisionEntry | null;
  /** Files that version needs but haven't synced yet ("Still syncing"). */
  missing_files: string[];
  can_keep: boolean;
}

/** `project_conflict_get`. */
export interface ConflictView {
  /** `copy`: a `project.json` conflict copy; `seq`: changed on disk while open. */
  source: 'copy' | 'seq';
  this: ConflictSide;
  other: ConflictSide;
}

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
  /** REF's selection (FR-REF-20, FR-PRJ-54). Sent only once the user changed it. */
  references?: DraftReferences | null;
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
  slots: CapabilitySlot[];
  warnings: string[];
}

/** A reference image input of a model (FR-PRV-31, R49). `min` is 1 for a required input;
 *  `array` = the input takes a list (`file_array`) rather than one file. */
export interface CapabilitySlot {
  name: string;
  role: RefRole | null;
  min: number;
  max: number;
  cost_affecting: boolean;
  array: boolean;
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

/** EXP grid input (manifest-schema §4.10, FR-EXP-06): px per square or cells. */
export interface ExportGrid {
  mode: 'px' | 'cells';
  source_px_per_square?: number | null;
  cols?: number | null;
  rows?: number | null;
}

export type ExportFormat = 'png' | 'jpeg' | 'webp';

/** The export settings the owner edits (EXP §9 `export_set_settings` input). */
export interface ExportSettingsInput {
  grid: ExportGrid;
  target_px_per_square: number | null;
  format: ExportFormat;
}

/** `export_get_settings` / `export_set_settings` result: the settings plus the derived source
 *  px per square. EDT reads the two px-per-square values (FR-EXP-26, FR-EDT-04). */
export interface ExportSettings extends Partial<ExportSettingsInput> {
  source_px_per_square: number | null;
  target_px_per_square: number | null;
}

/** `invalid_settings` limit (`detail.limit`): the bound the field broke. */
export type ExportLimit = { min: number } | { max: number } | { requires: string };

/** `export_plan` result (FR-EXP-24). `path` is relative to the project folder. */
export interface ExportPlan {
  width: number;
  height: number;
  scale: number;
  formats: { format: ExportFormat; available: boolean; reason: string | null }[];
  path: string;
  exists: { bytes: number; mtime: string } | null;
  est_peak_mib: number;
  warnings: { code: string; message: string }[];
}

export type ExportPhase = 'compose' | 'resize' | 'encode' | 'write';

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

// ── Region edit (RGN §9) ──────────────────────────────────────────────────────────────────────

/** PRV's generic cost (`Cost`): provider units, or "unknown". */
export interface CostValue {
  amount: number | 'unknown';
  unit: string;
}

export type JobState =
  | 'requested'
  | 'submitted'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'fetched'
  | 'closed'
  | 'failed'
  | 'canceled'
  | 'unknown'
  | 'lost';

/** `rgn_iterations_list` row (FR-RGN-40). The active job before it has candidates has
 *  `n`, `dir` and `slug` null. */
export interface RgnIteration {
  job_id: string;
  n: number | null;
  dir: string | null;
  slug: string | null;
  state: JobState;
  kept: number;
  returned: number;
  cost: CostValue | null;
  accepted_in_stack: boolean;
}

export interface RgnBlend {
  feather_px: number;
  colour_match: boolean;
}

/** `rgn_candidate_preview` (R16). `url` is the transient key's URL: the UI never builds hosts
 *  (EDT Appendix A). */
export interface RgnPreview {
  key: string;
  url: string;
  rect: RectPx;
  /** `colour_match_skipped`: under 1 % of the outer rect is unmasked (FR-RGN-26). */
  notices?: string[];
}

/** A manifest candidate (schema §4.7) as `project_get` returns it. */
export interface CandidateRecord {
  id: string;
  job_id: string;
  index: number;
  file: string;
  status: 'new' | 'accepted' | 'dismissed' | 'discarded' | 'trashed' | 'invalid';
  starred: boolean;
  trash_id: string | null;
  /** Set when the 30-day purge deleted the file (BL-23); `trash_id` then names the purged item. */
  purged_at?: string;
  width?: number;
  height?: number;
  /** Aspect differs from the sent one by > 1 % (FR-RGN-24). */
  aspect_warning?: boolean;
}

/** Core command name → [input, output]. Append new commands. */
export interface Commands {
  app_info: [Record<string, never>, AppInfo];
  library_get_config: [Record<string, never>, LibraryConfig];
  library_set_roots: [{ roots: string[] }, LibraryConfig];
  project_open: [{ path: string }, ProjectSnapshot];
  /** `force`: close although a save is still pending (FR-PRJ-21, `unsaved` otherwise). */
  project_close: [{ force?: boolean }, Record<string, never>];
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
  rgn_controls_evaluate: [
    { model_id: string; values: Record<string, unknown> },
    ControlsEvaluation,
  ];
  rgn_job_estimate: [{ project_id?: string; draft: JobDraft }, JobEstimate];
  rgn_spend_get: [Record<string, never>, Spend];
  /** A missing field keeps its value; null clears it. */
  rgn_spend_set: [{ cap?: number | null; threshold?: number | null }, Spend];
  rgn_job_submit: [
    { draft: JobDraft; estimate_id: string; confirmations: SpendNeed[] },
    { job_id: string },
  ];
  rgn_job_cancel: [{ job_id: string }, { result: 'canceled' | 'not_cancellable' }];
  rgn_job_retry_draft: [{ job_id: string }, JobDraft];
  rgn_job_dismiss: [{ job_id: string }, { trashed: number }];
  rgn_candidate_preview: [{ job_id: string; candidate_id: string; blend: RgnBlend }, RgnPreview];
  rgn_candidate_accept: [
    { job_id: string; candidate_id: string; blend: RgnBlend },
    { layer_id: string; revision_id: string },
  ];
  rgn_candidate_star: [
    { job_id: string; candidate_id: string; starred: boolean },
    Record<string, never>,
  ];
  rgn_candidate_discard: [{ job_id: string; candidate_id: string }, Record<string, never>];
  rgn_patch_retune: [
    { layer_id: string; blend: RgnBlend },
    { layer_id: string; revision_id: string },
  ];
  rgn_iterations_list: [Record<string, never>, RgnIteration[]];
  export_set_settings: [{ settings: ExportSettingsInput }, ExportSettings];
  export_plan: [{ settings: ExportSettingsInput }, ExportPlan];
  export_start: [{ settings: ExportSettingsInput; replace: boolean }, { export_id: string }];
  export_cancel: [{ export_id: string }, Record<string, never>];
  project_prefetch_original: [Record<string, never>, { ready?: boolean } | null];
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
  // reference-packs §9 (#51, #54)
  ref_list_packs: [Record<string, never>, PackCard[]];
  ref_pack_get: [{ pack_id: string }, Pack];
  ref_pack_create: [
    { name: string; paths?: string[]; suffix_if_taken?: boolean },
    { pack: Pack; items: PackAddItem[] },
  ];
  ref_pack_rename: [{ pack_id: string; name: string }, Pack];
  ref_pack_update: [
    { pack_id: string; notes?: string; roles?: Record<string, RefRole | null> },
    Pack,
  ];
  ref_pack_add_images: [{ pack_id: string; paths: string[] }, { items: PackAddItem[]; pack: Pack }];
  ref_pack_remove_images: [{ pack_id: string; files: string[] }, { item_id: string }];
  ref_pack_delete: [{ pack_id: string }, { item_id: string }];
  ref_trash_list: [Record<string, never>, RefTrashItem[]];
  ref_trash_restore: [{ id: string }, { path: string }];
  ref_resolve: [{ model_id: string; references: DraftReferences | null }, RefResolved];
  project_conflict_get: [Record<string, never>, ConflictView];
  project_resolve_conflict: [{ keep: ConflictKeep }, RevisionEntry];
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
  'rgn:job-state': {
    job_id: string;
    state: JobState;
    progress: number | null;
    cost: CostValue | null;
    error?: { code: string; hint?: string; message?: string } | null;
    /** FR-PRV-19 `long_running` (from 180 s). */
    long_running?: boolean;
  };
  'rgn:candidate-ready': { job_id: string; candidate_id: string; status: 'ready' | 'invalid' };
  'rgn:spend': { spend: number; cap: number | null };
  'project:original-ready': Record<string, never> | null;
  'export:progress': { export_id: string; phase: ExportPhase; fraction: number };
  'export:done': {
    export_id: string;
    path: string;
    width: number;
    height: number;
    bytes: number;
    format: ExportFormat;
    fallback: 'lossless' | null;
    record_saved: boolean;
  };
  'export:failed': { export_id: string; code: string; detail: unknown };
  'export:canceled': { export_id: string };
  'import:progress': { done: number; total: number };
  'project:save-failed': { reason: string; retrying: boolean };
  'project:conflict': Record<string, never>;
  'project:recovered': Record<string, never>;
  'project:thumb-updated': { project_id: string };
  'ref:packs-changed': Record<string, never> | null;
  'ref:add-progress': { done: number; total: number };
  'ref:thumb-updated': { pack_id: string };
  'ref:trash-purged': { count: number; bytes: number };
}

// ── Reference packs (REF §9, #51, #54; shapes final per R50, pf-core `references/`) ──────────

/** A per-image role (FR-REF-11); `null` = none. */
export type RefRole = 'style' | 'structure' | 'identity';

export type PackBadge = 'not_ready' | 'not_available_offline' | 'conflict';

/** Something a listing did that the user is told about (R50): `adopted`, `new_id`,
 *  `conflict_resolved`, `pack_json_replaced`. */
export interface RefNotice {
  code: string;
  message: string;
  /** The item a `ref_resolve` notice is about. */
  key?: string;
}

/** A packs-home card (FR-REF-06), from `pack.json`, the thumbnail and a directory listing. */
export interface PackCard {
  /** The pack id, or the folder path when the pack has no usable `pack.json` (record id). */
  id: string;
  /** null while the pack has no `pack.json` (`not_ready`, FR-REF-04): it can't be opened or
   *  attached until a later listing adopts it. */
  pack_id: string | null;
  /** The folder name (OQ-REF-01). */
  name: string;
  /** Absolute pack folder. */
  path: string;
  image_count: number;
  /** The first line of the notes ("" when none). */
  notes_first_line: string;
  /** `References/<pack>/.pforge/thumb.jpg` on the `pf-tiles` scheme, or null (placeholder). */
  thumb_url: string | null;
  badges: PackBadge[];
  notices: RefNotice[];
}

export type PackImageState = 'ok' | 'cloud_only' | 'corrupt' | 'unsupported';

/** A pack image (`ref_pack_get`, FR-REF-01, 14). */
export interface PackImage {
  file: string;
  /** Absolute path. */
  path: string;
  bytes: number;
  mtime: string;
  w?: number;
  h?: number;
  role: RefRole | null;
  state: PackImageState;
  /** Thumbnail on the `pf-tiles` scheme (EDT `library` keys, FR-REF-14); absent = none. */
  thumb_url?: string;
}

export interface Pack {
  pack_id: string;
  name: string;
  /** Absolute pack folder. */
  path: string;
  notes: string;
  images: PackImage[];
}

/** One source of `ref_pack_create` / `ref_pack_add_images` (FR-REF-07, 08). */
export type PackAddItem =
  | { status: 'added'; source: string; file: string }
  | { status: 'already_in_pack'; source: string; file: string }
  | { status: 'skipped'; source: string; reason: SkipReason };

/** A Recently deleted row (FR-REF-12). */
export interface RefTrashItem {
  id: string;
  kind: 'pack' | 'images';
  pack_id: string | null;
  pack_name: string;
  files: string[];
  trashed_at: string;
  /** When it will be purged (30 days). */
  purge_at: string;
}

/** The draft's `references` (schema §4.5). */
export interface DraftReferences {
  packs: { pack_id: string; name: string }[];
  items: { key: string; on: boolean; role: RefRole | null }[];
}

/** A model reference slot as `ref_resolve` reports it; `on` = items ticked in it. */
export interface RefSlot {
  name: string;
  role: RefRole | null;
  min: number;
  max: number;
  cost_affecting: boolean;
  on: number;
}

export type RefItemState = 'ok' | 'cloud_only' | 'corrupt' | 'missing';

/** An available item (FR-REF-16). Badges: `cloud`, `missing`, `corrupt`, `duplicate`
 *  (slice 6 adds `changed_since:<NNN>`). */
export interface RefItem {
  /** `project:<file>` or `pack:<pack_id>:<file>`. */
  key: string;
  source: 'project' | 'pack';
  pack_id: string | null;
  pack_name: string | null;
  file: string;
  role: RefRole | null;
  /** The slot it is sent in. */
  slot: string;
  on: boolean;
  state: RefItemState;
  badges: string[];
  /** False for missing and corrupt items (they can't be ticked). */
  checkable: boolean;
  thumb_key?: string;
  thumb_url?: string;
}

/** `ref_resolve` (FR-REF-15..19, 27). */
export interface RefResolved {
  /** False when the model has no slots: the block is hidden (FR-REF-15). */
  visible: boolean;
  slots: RefSlot[];
  items: RefItem[];
  notices: RefNotice[];
  /** The reconciled draft value, for the draft save (FR-REF-20). */
  references: DraftReferences;
  /** Every slot is at or over its `min` (FR-REF-18). */
  ready: boolean;
  /** Read-only project (FR-REF-27). */
  read_only: boolean;
}
