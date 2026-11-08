// The spend confirm dialog's logic (FR-RGN-14/15/16, §7): what it shows, when it can confirm,
// whether Enter confirms, and the confirmations sent with submit. Pure.
import type { SpendNeed, JobEstimate } from '../../api';

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
  /** "N unstarred candidates of the previous job go to the trash". */
  trashCount: number;
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

export function initialForm(est: JobEstimate, threshold: number | null): ConfirmForm {
  return {
    cap: est.cap === null ? '' : String(est.cap),
    threshold: threshold === null ? '' : String(threshold),
    second: false,
  };
}

export function confirmPlan(
  est: JobEstimate,
  stored: { cap: number | null; threshold: number | null },
  form: ConfirmForm,
  trashCount: number,
): ConfirmPlan {
  const setup = est.needs.includes('set_cap') || stored.cap === null || stored.threshold === null;
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
    trashCount,
    references: est.references ?? 0,
    canConfirm: blocker === null,
    blocker,
    saveSpend,
    confirmations,
  };
}
