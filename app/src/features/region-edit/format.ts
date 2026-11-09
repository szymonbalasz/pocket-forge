// Labels for the Generate panel, the tracker and the status bar. Pure.
import { sentence, type CostValue, type JobState, type RegionFit } from '../../api';

/** FR-RGN-05: "sent unresampled" or "patch N.N× softer" (outer ÷ sent, one decimal). */
export function fitLabel(fit: Pick<RegionFit, 'outer' | 'sent_w' | 'sent_h'>): string {
  if (fit.sent_w === fit.outer.w && fit.sent_h === fit.outer.h) return 'sent unresampled';
  const ratio = Math.max(fit.outer.w / fit.sent_w, fit.outer.h / fit.sent_h);
  return `patch ${ratio.toFixed(1)}× softer`;
}

// signDisplay 'negative': a zero spend (or one that rounds to 0) never shows as "-0" (walk D6).
const num = new Intl.NumberFormat('en-US', { maximumFractionDigits: 2, signDisplay: 'negative' });

export const amountLabel = (n: number, unit: string): string => `${num.format(n)} ${unit}`;

/** A cost in provider units, or "unknown" (FR-RGN-12). */
export function costLabel(cost: CostValue | null | undefined): string {
  if (!cost || cost.amount === 'unknown') return 'unknown';
  return amountLabel(cost.amount, cost.unit);
}

export function elapsedLabel(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export const STATE_LABEL: Record<JobState, string> = {
  requested: 'Requested',
  submitted: 'Submitted',
  queued: 'Queued',
  running: 'Running',
  succeeded: 'Fetching',
  fetched: 'Ready',
  closed: 'Closed',
  failed: 'Failed',
  canceled: 'Canceled',
  unknown: 'Unknown (offline)',
  lost: 'Lost',
};

export const iterationNumber = (n: number): string => String(n).padStart(3, '0');

/** FR-RGN-20: the normalised error + hint; `insufficient_balance` gets its own message. */
export function jobErrorText(e: { code: string; hint?: string; message?: string }): string {
  if (e.code === 'insufficient_balance')
    return 'Not enough provider balance for this job. Top up your Scenario account, then retry.';
  // Job rows come from events, not ApiError: sentence case here (BL-27).
  const base = sentence(e.message && e.message !== e.code ? e.message : e.code.replace(/_/g, ' '));
  return e.hint ? `${base}. ${e.hint}` : base;
}

/** R108: what a lost job's strip says. With its handle kept it may still finish at the
 *  provider; without one its result can't be recovered. */
export function lostText(handleKept: boolean): string {
  return handleKept
    ? 'The job may still finish at the provider.'
    : 'Its result can’t be recovered.';
}

/** R108: the possible-charge hint of a lost job: the core's hint, else a generic one. */
export function chargeHint(e: { hint?: string } | null | undefined): string {
  return e?.hint || 'The job may have been charged; check the provider before retrying.';
}

/** Messages for command errors the panel shows inline. */
export function errorText(code: string, message: string): string {
  switch (code) {
    case 'region_too_small':
    case 'region_too_large':
    case 'out_of_bounds':
      return message;
    case 'mask_invalid':
      // Owner 2026-11-08 (MASK_HINT in store.ts; not imported: store imports this module).
      return 'Paint a mask inside the region to choose what changes.';
    case 'original_unavailable':
      return 'The original is still loading. Try again in a moment.';
    case 'corrupt':
      return 'The original can’t be decoded (the file may be damaged).';
    case 'offline':
      return 'The provider can’t be reached (offline).';
    case 'auth':
    case 'not_configured':
      return 'The provider isn’t set up.';
    case 'job_active':
      return 'A job is already running in this project.';
    case 'read_only':
      return 'The project is read-only.';
    case 'invalid_candidate':
      // Walk-p4r L6: the core's text is a clause; shown as a sentence.
      return /[.!?]$/.test(message) ? message : `${message}.`;
    case 'repick_unavailable':
      return 'Only the newest iteration can be accepted (after undoing its accept). Compare, star and discard still work.';
    case 'estimate_stale':
      return 'The settings changed since the estimate; estimating again.';
    case 'io':
      // Walk owner-UX L2: never the OS text (paths, "os error 3").
      return 'Couldn’t read the project’s files. Check the project folder, then try again.';
    default:
      return message || code;
  }
}
