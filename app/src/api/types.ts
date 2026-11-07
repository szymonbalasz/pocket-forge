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

export interface ProviderStatus {
  mode: 'fake' | 'scenario';
  configured: boolean;
  connectivity: string;
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
  edt_tiles_open: [{ project_id: string; image_id: string; rebuild?: boolean }, TilesDescriptor];
  export_get_settings: [Record<string, never>, ExportSettings];
  rgn_region_fit: [
    { project_id: string; inner: RectPx; margin?: number; model_id?: string | null },
    RegionFit,
  ];
  edt_mask_read: [{ project_id: string; mask_id: string }, MaskRead];
  project_restore_revision: [{ rev_id: string }, RevisionEntry];
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
}
