// The spend confirm dialog's logic (FR-RGN-14/15/16, §7): what it shows, when it can confirm,
// whether Enter confirms, and the confirmations sent with submit. Pure.
import type { SpendNeed, JobEstimate } from '../../api';

/** What the dialog needs from an estimate: RGN's, or GEN's (no region fit, FR-GEN-21). */
export type SpendEstimate = Omit<JobEstimate, 'fit' | 'candidate_count'>;

export interface ConfirmForm {
  /** Cap and threshold as typed (setup, or the explicit "raise cap" field). */
  cap: string;
  threshold: string;
  /** The second explicit confirmation (over threshold or unknown cost). */
  second: boolean;
}

export interface ConfirmPlan {
  cost: number | 'unknown';
  unit: string;
  spend: number;
  /** Session spend after this job (unknown cost counts as 0 until it is known). */
  spendAfter: number;
  cap: number | null;
  threshold: number | null;
  /** FR-RGN-16: no cap or threshold yet → ask for both. */
  setup: boolean;
  /** FR-RGN-14: spend + estimate > cap. */
  overCap: boolean;
  /** FR-RGN-15. */
  overThreshold: boolean;
  unknownCost: boolean;
  /** Over threshold or unknown cost: a second explicit confirmation is required. */
  needsSecond: boolean;
  /** §7: Enter never confirms an over-threshold (or unknown-cost) job. */
  enterConfirms: boolean;
  /** References sent with the job: "with N references" (FR-RGN-45). */
  references: number;
  canConfirm: boolean;
  /** Why it can't confirm yet. */
  blocker: string | null;
  /** Settings to store before submit (setup or a raised cap), else null. */
  saveSpend: { cap: number; threshold: number } | null;
  confirmations: SpendNeed[];
}

const parse = (s: string): number | null => {
  if (s.trim() === '') return null;
  const n = Number(s);
  return Number.isFinite(n) && n >= 0 ? n : null;
};

export function initialForm(est: SpendEstimate, threshold: number | null): ConfirmForm {
  return {
    cap: est.cap === null ? '' : String(est.cap),
    threshold: threshold === null ? '' : String(threshold),
    second: false,
  };
}

/** FR-RGN-45: the references the job sends. From the current selection (`onKeys`, REF's ticked
 *  items), not the estimate's count: an estimate is reused while only non-cost-affecting slots
 *  change (FR-REF-24). Items the estimate found identical to an earlier one are sent once.
 *  `null` (no resolved block) → the estimate's count. */
export function referencesSent(est: SpendEstimate, onKeys: string[] | null): number {
  if (!onKeys) return est.references ?? 0;
  const once = new Set(
    (est.reference_notices ?? []).filter((n) => n.code === 'duplicate').map((n) => n.key),
  );
  return onKeys.filter((k) => !once.has(k)).length;
}

export function confirmPlan(
  est: SpendEstimate,
  stored: { cap: number | null; threshold: number | null },
  form: ConfirmForm,
  onKeys: string[] | null = null,
): ConfirmPlan {
  // From the stored limits, not `est.needs`: a reused estimate can predate them (walk-61 L3).
  const setup = stored.cap === null || stored.threshold === null;
  const cost = est.cost.amount;
  const known = cost === 'unknown' ? 0 : cost;
  const typedCap = parse(form.cap);
  const typedThreshold = parse(form.threshold);
  const cap = setup || typedCap !== null ? typedCap : stored.cap;
  const threshold = setup ? typedThreshold : (stored.threshold ?? typedThreshold);
  const spendAfter = est.spend + known;
  const overCap = cap !== null && spendAfter > cap;
  const unknownCost = cost === 'unknown';
  const overThreshold = !unknownCost && threshold !== null && cost > threshold;
  const needsSecond = overThreshold || unknownCost;

  let blocker: string | null = null;
  if (setup && (typedCap === null || typedThreshold === null))
    blocker = 'Set a session spend cap and a per-job threshold first.';
  else if (overCap) blocker = 'This job goes over the spend cap. Raise the cap to continue.';
  else if (needsSecond && !form.second) blocker = 'Confirm the cost a second time.';

  const capChanged = cap !== null && cap !== stored.cap;
  const saveSpend =
    (setup || capChanged) && cap !== null && threshold !== null ? { cap, threshold } : null;

  const confirmations: SpendNeed[] = [];
  if (setup) confirmations.push('set_cap');
  if (overThreshold && form.second) confirmations.push('threshold');
  if (unknownCost && form.second) confirmations.push('unknown_cost');

  return {
    cost,
    unit: est.cost.unit,
    spend: est.spend,
    spendAfter,
    cap,
    threshold,
    setup,
    overCap,
    overThreshold,
    unknownCost,
    needsSecond,
    enterConfirms: !needsSecond,
    references: referencesSent(est, onKeys),
    canConfirm: blocker === null,
    blocker,
    saveSpend,
    confirmations,
  };
}

/** BL-36: a fresh estimate may be submitted under the plan the DM confirmed: within its cap, no
 *  dearer than the cost shown, and needing no confirmation that wasn't given. */
export function withinConfirmed(fresh: SpendEstimate, plan: ConfirmPlan): boolean {
  const cost = fresh.cost.amount;
  if (plan.cap === null) return false;
  if (fresh.spend + (cost === 'unknown' ? 0 : cost) > plan.cap) return false;
  if (!fresh.needs.every((n) => plan.confirmations.includes(n))) return false;
  return plan.cost === 'unknown' || (cost !== 'unknown' && cost <= plan.cost);
}
