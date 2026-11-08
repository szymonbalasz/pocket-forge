// The spend confirm dialog (§7, FR-RGN-14/15/16): estimate, spend after this job, cap, threshold
// or unknown-cost warning, the trash notice, first-submit cap setup and the explicit "raise cap"
// field. The only blocking modal (ui-guidelines). Esc closes; Enter never confirms an
// over-threshold or unknown-cost job.
import { useMemo, useRef, useState, type FormEvent } from 'react';
import { Checkbox, Dialog } from 'radix-ui';

import { IconCheck } from '../../shell/icons';
import { referencesOnKeys, useReferencesBlock } from '../references';
import { confirmPlan, initialForm, type ConfirmForm } from './confirm';
import { amountLabel, errorText } from './format';
import { rgn, useRgn } from './store';
import { trashOnSubmit } from './tracker';

export function ConfirmDialog() {
  const s = useRgn((x) => x);
  const est = s.estimate.result;
  const open = s.confirmOpen && !!est;
  return (
    <Dialog.Root open={open} onOpenChange={(o) => !o && rgn.closeConfirm()}>
      {open && est && <ConfirmBody key={est.estimate_id} />}
    </Dialog.Root>
  );
}

function ConfirmBody() {
  const s = useRgn((x) => x);
  const est = s.estimate.result!;
  const stored = { cap: s.spend?.cap ?? est.cap, threshold: s.spend?.threshold ?? null };
  const live = { ...est, spend: s.spend?.spend ?? est.spend };
  const [form, setForm] = useState<ConfirmForm>(() => initialForm(live, stored.threshold));
  const trash = trashOnSubmit(s.iterations, s.candidates);
  // FR-RGN-45: the count follows the block's selection, not the (reused) estimate.
  const refsBlock = useReferencesBlock();
  const onKeys = useMemo(() => referencesOnKeys(refsBlock), [refsBlock]);
  const plan = useMemo(
    () => confirmPlan(live, stored, form, trash, onKeys),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [est, s.spend, form, trash, onKeys],
  );
  const okRef = useRef<HTMLButtonElement>(null);
  const capRef = useRef<HTMLInputElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  const confirm = () => {
    if (plan.canConfirm && !s.submitting) void rgn.confirm(plan);
  };
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    // Enter in a field: only a job within the threshold, with a known cost, confirms.
    if (plan.enterConfirms) confirm();
  };
  const unit = plan.unit;
  const money = (n: number | null) => (n === null ? '—' : amountLabel(n, unit));

  return (
    <Dialog.Portal>
      <Dialog.Overlay className="rgn-overlay" />
      <Dialog.Content
        className="rgn-dialog"
        data-testid="rgn-confirm"
        data-over-cap={plan.overCap ? 'true' : 'false'}
        data-over-threshold={plan.overThreshold ? 'true' : 'false'}
        data-unknown-cost={plan.unknownCost ? 'true' : 'false'}
        data-setup={plan.setup ? 'true' : 'false'}
        onOpenAutoFocus={(e) => {
          e.preventDefault();
          // Default focus: the field to fill, else Cancel when a second confirmation is
          // needed (so Enter can't confirm), else OK.
          (plan.setup || plan.overCap
            ? capRef
            : plan.needsSecond
              ? cancelRef
              : okRef
          ).current?.focus();
        }}
      >
        <Dialog.Title className="rgn-dialog-title">Submit this job?</Dialog.Title>
        <Dialog.Description className="muted small">
          Spend is checked against your session cap before anything is sent.
        </Dialog.Description>
        <form onSubmit={onSubmit}>
          <dl className="rgn-dialog-table tabular">
            <dt>Estimate</dt>
            <dd data-testid="rgn-confirm-estimate">
              {plan.cost === 'unknown' ? 'unknown' : amountLabel(plan.cost, unit)}
              {plan.references > 0 && (
                <span data-testid="rgn-confirm-references">
                  {' '}
                  with {plan.references} reference{plan.references === 1 ? '' : 's'}
                </span>
              )}
            </dd>
            <dt>Session spend after this job</dt>
            <dd data-testid="rgn-confirm-spend">
              {money(plan.spendAfter)}
              {plan.unknownCost ? ' + unknown' : ''}
            </dd>
            <dt>Spend cap</dt>
            <dd>{money(plan.cap)}</dd>
            <dt>Per-job threshold</dt>
            <dd>{money(plan.threshold)}</dd>
          </dl>

          {plan.setup && (
            <fieldset className="rgn-dialog-setup">
              <legend>Set your spend limits (asked once, stored on this machine)</legend>
              <label>
                Session spend cap ({unit})
                <input
                  ref={capRef}
                  type="number"
                  min={0}
                  className="tabular"
                  data-testid="rgn-confirm-cap"
                  value={form.cap}
                  onChange={(e) => setForm({ ...form, cap: e.target.value })}
                />
              </label>
              <label>
                Ask twice above ({unit} per job)
                <input
                  type="number"
                  min={0}
                  className="tabular"
                  data-testid="rgn-confirm-threshold"
                  value={form.threshold}
                  onChange={(e) => setForm({ ...form, threshold: e.target.value })}
                />
              </label>
            </fieldset>
          )}

          {!plan.setup && (plan.overCap || form.cap !== String(stored.cap ?? '')) && (
            <div className="rgn-dialog-warn" data-testid="rgn-confirm-over-cap">
              {plan.overCap && (
                <p>
                  This job would bring the session to {money(plan.spendAfter)}, over the cap of{' '}
                  {money(plan.cap)}.
                </p>
              )}
              <label>
                Raise cap to ({unit})
                <input
                  ref={capRef}
                  type="number"
                  min={0}
                  className="tabular"
                  data-testid="rgn-confirm-cap"
                  value={form.cap}
                  onChange={(e) => setForm({ ...form, cap: e.target.value })}
                />
              </label>
            </div>
          )}

          {plan.needsSecond && (
            <div className="rgn-dialog-warn" data-testid="rgn-confirm-warning">
              <p>
                {plan.unknownCost
                  ? 'The provider can’t tell what this job costs. It could be more than you expect.'
                  : `This job costs more than your per-job threshold of ${money(plan.threshold)}.`}
              </p>
              <label className="rgn-check">
                <Checkbox.Root
                  className="rgn-checkbox"
                  data-testid="rgn-confirm-second"
                  checked={form.second}
                  onCheckedChange={(v) => setForm({ ...form, second: v === true })}
                >
                  <Checkbox.Indicator>
                    <IconCheck size={12} />
                  </Checkbox.Indicator>
                </Checkbox.Root>
                {plan.unknownCost ? 'Yes, submit with an unknown cost' : 'Yes, spend this much'}
              </label>
            </div>
          )}

          {plan.trashCount > 0 && (
            <p className="muted small" data-testid="rgn-confirm-trash">
              {plan.trashCount} unstarred candidate{plan.trashCount === 1 ? '' : 's'} of the
              previous job go{plan.trashCount === 1 ? 'es' : ''} to the trash.
            </p>
          )}

          {s.confirmError && (
            <p className="error small" data-testid="rgn-confirm-error">
              {errorText(s.confirmError.code, s.confirmError.message)}
            </p>
          )}
          {plan.blocker && <p className="muted small">{plan.blocker}</p>}

          <div className="rgn-dialog-buttons">
            <Dialog.Close asChild>
              <button type="button" ref={cancelRef} data-testid="rgn-confirm-cancel">
                Cancel
              </button>
            </Dialog.Close>
            <button
              type="button"
              ref={okRef}
              className="primary"
              data-testid="rgn-confirm-ok"
              disabled={!plan.canConfirm || s.submitting}
              onClick={confirm}
              onKeyDown={(e) => {
                // Enter on the focused button must not confirm a job that needs asking twice.
                if (e.key === 'Enter' && !plan.enterConfirms) e.preventDefault();
              }}
            >
              {s.submitting ? 'Submitting…' : 'Submit'}
            </button>
          </div>
        </form>
      </Dialog.Content>
    </Dialog.Portal>
  );
}
