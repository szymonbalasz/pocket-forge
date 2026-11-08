// Pure view logic for the Export panel (EXP §7): form → settings, the plan line, the Replace
// note, Start's state and error wording. No commands here; the store calls the core.
import type {
  ExportFormat,
  ExportPhase,
  ExportPlan,
  ExportSettings,
  ExportSettingsInput,
  ReadOnlyReason,
} from '../../api';

/** The panel's raw input text, per field (both grid modes keep their values). */
export interface ExportForm {
  mode: 'px' | 'cells';
  source: string;
  cols: string;
  rows: string;
  target: string;
  format: ExportFormat;
}

export const emptyForm: ExportForm = {
  mode: 'px',
  source: '',
  cols: '',
  rows: '',
  target: '',
  format: 'png',
};

const text = (v: number | null | undefined) => (v === null || v === undefined ? '' : String(v));

/** The form for settings loaded from the core (`export_get_settings`, FR-EXP-25). */
export function formOf(s: ExportSettings): ExportForm {
  return {
    mode: s.grid?.mode ?? 'px',
    source: text(s.grid?.source_px_per_square),
    cols: text(s.grid?.cols),
    rows: text(s.grid?.rows),
    target: text(s.target_px_per_square),
    format: s.format ?? 'png',
  };
}

export type FieldName = 'source' | 'cols' | 'rows' | 'target';

export interface FieldError {
  /** The form field, or null for a plan-level limit (scale, output size). */
  field: FieldName | null;
  message: string;
}

function parse(
  field: FieldName,
  raw: string,
  integer: boolean,
): { value: number | null } | { error: FieldError } {
  const t = raw.trim();
  if (t === '') return { value: null };
  const v = Number(t);
  if (!Number.isFinite(v)) return { error: { field, message: 'Enter a number' } };
  if (integer && !Number.isInteger(v)) return { error: { field, message: 'Whole number' } };
  return { value: v };
}

/** The settings for the form, or the first field that isn't a number (checked before the core).
 *  Without a grid value the target is disabled and sent as null (FR-EXP-07). */
export function settingsOf(
  form: ExportForm,
): { settings: ExportSettingsInput } | { error: FieldError } {
  const hasSource = formHasSource(form);
  const fields: [FieldName, string, boolean][] = [
    ['source', form.source, false],
    ['cols', form.cols, true],
    ['rows', form.rows, true],
    ['target', form.target, true],
  ];
  const v: Partial<Record<FieldName, number | null>> = {};
  for (const [name, raw, int] of fields) {
    // Only the active grid mode's fields can block.
    const active =
      (name === 'target' && hasSource) ||
      (form.mode === 'px' ? name === 'source' : name === 'cols' || name === 'rows');
    const r = parse(name, raw, int);
    if ('error' in r) {
      if (active) return r;
      v[name] = null;
    } else v[name] = r.value;
  }
  return {
    settings: {
      grid: {
        mode: form.mode,
        source_px_per_square: v.source ?? null,
        cols: v.cols ?? null,
        rows: v.rows ?? null,
      },
      target_px_per_square: hasSource ? (v.target ?? null) : null,
      format: form.format,
    },
  };
}

/** Whether the target field is usable: the active grid mode gives a source (FR-EXP-07). */
export function formHasSource(form: ExportForm): boolean {
  return (form.mode === 'px' ? form.source : form.cols).trim() !== '';
}

const FIELD_OF: Record<string, FieldName | null> = {
  source_px_per_square: 'source',
  cols: 'cols',
  rows: 'rows',
  target_px_per_square: 'target',
};

/** `invalid_settings{field, limit}` in words, attached to its form field (FR-EXP-06..08). */
export function invalidSettingsError(field: string | undefined, limit: unknown): FieldError {
  const l = (limit ?? {}) as Partial<Record<'min' | 'max' | 'requires', unknown>>;
  const f = FIELD_OF[field ?? ''] ?? null;
  if (field === 'scale') return { field: 'target', message: `At most ${String(l.max)}× upscale` };
  if (field === 'width' || field === 'height')
    return {
      field: 'target',
      message:
        l.max !== undefined
          ? `Output would be over ${String(l.max)} px`
          : 'Output would be under 1 px',
    };
  if (l.requires !== undefined) return { field: f, message: 'Set the grid first' };
  if (l.min !== undefined) return { field: f, message: `At least ${String(l.min)}` };
  if (l.max !== undefined) return { field: f, message: `At most ${String(l.max)}` };
  return { field: f, message: 'Out of range' };
}

function scaleText(scale: number): string {
  return `${Number(scale.toFixed(3))}×`;
}

/** "4200×5600 → 2100×2800 · 0.5× · exports/Crypt_70px.png", or "4200×5600 · Full resolution · …"
 *  (EXP §7). */
export function planLine(sourceW: number, sourceH: number, plan: ExportPlan): string {
  const src = `${sourceW}×${sourceH}`;
  const size =
    plan.scale === 1
      ? `${src} · Full resolution`
      : `${src} → ${plan.width}×${plan.height} · ${scaleText(plan.scale)}`;
  return `${size} · ${plan.path}`;
}

/** 12.3 MB, 840 KB, 512 B. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 365 * 86400],
  ['month', 30 * 86400],
  ['day', 86400],
  ['hour', 3600],
  ['minute', 60],
];

/** "2 days ago", "yesterday", "now". */
export function ago(iso: string, now: number): string {
  const secs = Math.round((Date.parse(iso) - now) / 1000);
  const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
  for (const [unit, size] of UNITS)
    if (Math.abs(secs) >= size) return rtf.format(Math.round(secs / size), unit);
  return rtf.format(0, 'second');
}

export function fileNameOf(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** "Replaces Crypt_70px.png (12.3 MB, 2 days ago)" when the destination exists (FR-EXP-15). */
export function replaceNote(plan: ExportPlan, now: number): string | null {
  if (!plan.exists) return null;
  return `Replaces ${fileNameOf(plan.path)} (${formatBytes(plan.exists.bytes)}, ${ago(plan.exists.mtime, now)})`;
}

const READ_ONLY_TEXT: Record<ReadOnlyReason, string> = {
  newer_version: 'Read-only: made by a newer Pocket Forge',
  original_changed: 'Read-only: the original image changed',
  original_missing: 'Read-only: the original image is missing',
};

export type OriginalState =
  { kind: 'loading' } | { kind: 'ready' } | { kind: 'failed'; message: string };

export interface StartState {
  enabled: boolean;
  label: 'Export' | 'Replace';
  /** Why Start is disabled (shown under it), or null. */
  reason: string | null;
}

/** Start's label and whether it can run (FR-EXP-15, FR-EXP-28, FR-EXP-29). */
export function startState(input: {
  hasProject: boolean;
  readOnly: ReadOnlyReason | null;
  original: OriginalState;
  plan: ExportPlan | null;
  format: ExportFormat;
  running: boolean;
  blocked: boolean;
}): StartState {
  const label = input.plan?.exists ? 'Replace' : 'Export';
  const no = (reason: string | null): StartState => ({ enabled: false, label, reason });
  if (!input.hasProject) return no('Open a project to export');
  if (input.readOnly) return no(READ_ONLY_TEXT[input.readOnly]);
  if (input.running) return no(null);
  if (input.original.kind === 'loading') return no('Loading original…');
  if (input.original.kind === 'failed') return no(input.original.message);
  if (input.blocked || !input.plan) return no(null);
  const fmt = input.plan.formats.find((f) => f.format === input.format);
  if (fmt && !fmt.available) return no(fmt.reason);
  return { enabled: true, label, reason: null };
}

const ERROR_TEXT: Record<string, string> = {
  locked: 'The file is in use (VTT or sync?)',
  busy: 'An export is already running for this project',
  exists: 'The file exists; choose Replace',
  original_unavailable: 'The original is still loading',
  read_only: 'The project is read-only',
  conflict: "The project changed elsewhere; the settings weren't saved",
  layer_unreadable: "A layer file can't be read, so nothing was exported",
  disk_full: 'The disk is full',
  out_of_memory: 'Not enough memory for this export',
  encode_failed: "The image couldn't be encoded",
  too_large: 'Too large for this format',
  dest_unavailable: "The library folder isn't available (offline or not writable)",
  no_project: 'No project is open',
};

/** The message for an export error code (EXP §7 Error). */
export function errorText(code: string, fallback?: string): string {
  return ERROR_TEXT[code] ?? fallback ?? code;
}

export const PHASE_TEXT: Record<string, string> = {
  compose: 'Flattening',
  resize: 'Resizing',
  encode: 'Encoding',
  write: 'Writing',
};

/** Each phase's share of the whole export, in run order. PNG and JPEG at full resolution encode
 *  while they flatten, so `compose` covers the encode there. Rough shares from the walkthrough
 *  timings; they only have to keep the bar moving forward. */
export type PhaseWeights = [ExportPhase, number][];

export function phaseWeights(rescale: boolean, format: ExportFormat): PhaseWeights {
  if (!rescale)
    return format === 'webp'
      ? [
          ['compose', 0.2],
          ['encode', 0.75],
          ['write', 0.05],
        ]
      : [
          ['compose', 0.95],
          ['write', 0.05],
        ];
  return format === 'webp'
    ? [
        ['compose', 0.2],
        ['resize', 0.2],
        ['encode', 0.55],
        ['write', 0.05],
      ]
    : [
        ['compose', 0.35],
        ['resize', 0.25],
        ['encode', 0.35],
        ['write', 0.05],
      ];
}

/** One 0–1 fraction for the whole export from a phase's own fraction (EXP §7: the bar never
 *  restarts per phase). A phase missing from the weights counts as the start of the next one. */
export function overallFraction(
  weights: PhaseWeights,
  phase: ExportPhase,
  fraction: number,
): number {
  const order: ExportPhase[] = ['compose', 'resize', 'encode', 'write'];
  const at = order.indexOf(phase);
  let done = 0;
  for (const [p, w] of weights) {
    const i = order.indexOf(p);
    if (i < at) done += w;
    else if (i === at) done += w * Math.min(1, Math.max(0, fraction));
  }
  return Math.min(1, done);
}
