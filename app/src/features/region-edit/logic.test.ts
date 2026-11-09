import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import type {
  CandidateRecord,
  ControlsEvaluation,
  ControlState,
  JobEstimate,
  RgnIteration,
} from '../../api';
import { confirmPlan, initialForm } from './confirm';
import {
  fromEvaluation,
  controlKind,
  costSignature,
  defaultsOf,
  missingRequired,
  sentValues,
  snap,
  withRemembered,
} from './controls';
import { EstimateDebouncer, ESTIMATE_DEBOUNCE_MS } from './estimate';
import { costLabel, errorText, fitLabel, jobErrorText } from './format';
import { sentWith } from './ConfirmDialog';
import { PATCHES_KEPT } from './store';
import { ALL_TYPES } from './testUtils';
import {
  canCancel,
  canDismiss,
  canRetry,
  defaultSelection,
  iterationCandidates,
  orderRows,
  refsTooltip,
  offersRetry,
  shownState,
  stripJobId,
} from './tracker';

const ctl = (name: string) => ALL_TYPES.controls.find((c) => c.name === name)!;
const ev = (patch: Partial<Record<string, Partial<ControlState>>> = {}): ControlState[] =>
  ALL_TYPES.controls.map((c) => ({
    name: c.name,
    type: c.type,
    role: c.role,
    min: c.min,
    max: c.max,
    step: c.step,
    default: c.default,
    cost_affecting: c.cost_affecting,
    visible: true,
    allowed: c.allowed,
    required: c.required,
    value: c.default,
    reset: false,
    ...patch[c.name],
  }));
const evaluation = (controls: ControlState[], unknown: string[] = []): ControlsEvaluation => ({
  model_id: ALL_TYPES.model_id,
  controls,
  required_missing: [],
  unknown,
  warnings: [],
  prompt_required: true,
  seed: true,
});

describe('capability-driven controls (Appendix A)', () => {
  test('FR-RGN-08 every fixture control type maps to its Appendix A control; images never shown', () => {
    const kinds = Object.fromEntries(ALL_TYPES.controls.map((c) => [c.name, controlKind(c)]));
    expect(kinds).toEqual({
      image: 'none',
      mask: 'none',
      text: 'prompt',
      num_samples: 'slider',
      guidance: 'slider',
      steps: 'number',
      strength: 'number',
      mode: 'select',
      style: 'select',
      hd: 'toggle',
      negative_prompt: 'text',
      seed: 'seed',
      reference: 'none',
    });
  });

  test('FR-RGN-08 defaults prefill every value control', () => {
    expect(defaultsOf(ALL_TYPES)).toEqual({
      num_samples: 2,
      guidance: 7.5,
      steps: 30,
      strength: 0.8,
      mode: 'fast',
      style: null,
      hd: false,
      negative_prompt: '',
    });
  });

  test('Appendix A step snapping and the known bound', () => {
    expect(snap(ctl('guidance'), 7.3)).toBe(7.5);
    expect(snap(ctl('guidance'), 99)).toBe(20);
    expect(snap(ctl('steps'), 3)).toBe(10);
    expect(snap(ctl('steps'), 1000)).toBe(1000);
    expect(snap(ctl('strength'), 1.4)).toBe(1);
  });

  test('FR-RGN-11 remembered values over the defaults; unknown names dropped with a notice', () => {
    const r = withRemembered(ALL_TYPES, { guidance: 9, mode: 'quality', other_model: 1 });
    expect(r.values).toMatchObject({ guidance: 9, mode: 'quality', other_model: 1, steps: 30 });
    expect(r.touched).toEqual(['guidance', 'mode', 'other_model']);
    const f = fromEvaluation(ALL_TYPES, r.values, r.touched, evaluation(ev(), ['other_model']));
    expect(f.values).not.toHaveProperty('other_model');
    expect(f.touched).toEqual(['guidance', 'mode']);
    // Walk-gen L4: the label, never the raw input name.
    expect(f.notices).toEqual(['“other model” isn’t a setting of this model, so it was dropped.']);
  });

  test('FR-RGN-33 a value the core reset (outside the effective allowed set) takes its value, with a notice', () => {
    const values = { ...defaultsOf(ALL_TYPES), mode: 'creative', style: 'photo' };
    const r = fromEvaluation(
      ALL_TYPES,
      values,
      ['mode', 'style'],
      evaluation(ev({ style: { allowed: ['painterly', 'ink'], value: 'painterly', reset: true } })),
    );
    expect(r.values.style).toBe('painterly');
    expect(r.touched).toEqual(['mode']);
    expect(r.notices[0]).toMatch(/^style: “photo”/);
    // Nothing reset: nothing changes.
    const ok = fromEvaluation(ALL_TYPES, { ...values, style: 'ink' }, ['style'], evaluation(ev()));
    expect(ok.notices).toEqual([]);
    expect(ok.values.style).toBe('ink');
  });

  test('FR-RGN-33 required controls are named; FR-RGN-09 the prompt when prompt_required', () => {
    const values = { ...defaultsOf(ALL_TYPES), mode: 'quality' };
    expect(missingRequired(ALL_TYPES, ev({ style: { required: true } }), values, '')).toEqual([
      'prompt',
      'style',
    ]);
    expect(missingRequired(ALL_TYPES, ev(), values, 'a kitchen')).toEqual([]);
    // Hidden required controls are not asked for.
    expect(
      missingRequired(ALL_TYPES, ev({ style: { required: true, visible: false } }), values, 'x'),
    ).toEqual([]);
  });

  test('FR-RGN-34 sent = visible controls that are required or touched; hidden ones never', () => {
    const values = { ...defaultsOf(ALL_TYPES), guidance: 9, strength: 0.5, style: 'ink' };
    const sent = sentValues(
      ALL_TYPES,
      ev({ strength: { visible: false }, style: { required: true } }),
      values,
      ['guidance', 'strength'],
    );
    expect(sent).toEqual({ guidance: 9, style: 'ink' });
  });

  test('FR-RGN-12 the cost signature changes only with model, sent size or cost-affecting controls', () => {
    const base = costSignature('m', { w: 512, h: 512 }, ALL_TYPES, { num_samples: 2 });
    expect(costSignature('m', { w: 512, h: 512 }, ALL_TYPES, { num_samples: 2, guidance: 3 })).toBe(
      base,
    );
    expect(costSignature('m', { w: 512, h: 512 }, ALL_TYPES, { num_samples: 3 })).not.toBe(base);
    expect(costSignature('m', { w: 512, h: 520 }, ALL_TYPES, { num_samples: 2 })).not.toBe(base);
    expect(costSignature('n', { w: 512, h: 512 }, ALL_TYPES, { num_samples: 2 })).not.toBe(base);
    expect(
      costSignature('m', { w: 512, h: 512 }, ALL_TYPES, { num_samples: 2, hd: true }),
    ).not.toBe(base);
  });
});

describe('FR-RGN-12 estimate debounce', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  test('FR-RGN-12 estimates 500 ms after the last cost-affecting change, never for an unchanged signature', () => {
    const run = vi.fn();
    const d = new EstimateDebouncer(run);
    d.update('a');
    vi.advanceTimersByTime(ESTIMATE_DEBOUNCE_MS - 1);
    d.update('b'); // restarts the wait
    vi.advanceTimersByTime(ESTIMATE_DEBOUNCE_MS - 1);
    expect(run).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(run).toHaveBeenCalledExactlyOnceWith('b');
    d.update('b'); // prompt edits etc.: same signature
    vi.advanceTimersByTime(ESTIMATE_DEBOUNCE_MS * 4);
    expect(run).toHaveBeenCalledOnce();
    d.force(); // estimate_stale
    vi.advanceTimersByTime(ESTIMATE_DEBOUNCE_MS);
    expect(run).toHaveBeenCalledTimes(2);
    d.update(null); // no estimate possible
    d.update('b');
    vi.advanceTimersByTime(ESTIMATE_DEBOUNCE_MS);
    expect(run).toHaveBeenCalledTimes(3);
  });
});

describe('labels', () => {
  test('walk owner-UX L2: an io error never shows OS text', () => {
    const t = errorText(
      'io',
      'Reading mask 01M6: The system cannot find the path specified. (os error 3)',
    );
    expect(t).not.toMatch(/os error|01M6/);
  });

  test('FR-RGN-05 fit label: sent unresampled, or patch N.N× softer (outer ÷ sent)', () => {
    const outer = { x: 0, y: 0, w: 3000, h: 1500 };
    expect(fitLabel({ outer, sent_w: 3000, sent_h: 1500 })).toBe('sent unresampled');
    expect(fitLabel({ outer, sent_w: 2048, sent_h: 1024 })).toBe('patch 1.5× softer');
    expect(fitLabel({ outer: { x: 0, y: 0, w: 500, h: 500 }, sent_w: 504, sent_h: 504 })).toBe(
      'patch 1.0× softer',
    );
  });

  test('FR-RGN-12 cost in provider units, or unknown', () => {
    expect(costLabel({ amount: 1234.5, unit: 'CU' })).toBe('1,234.5 CU');
    expect(costLabel({ amount: 'unknown', unit: 'CU' })).toBe('unknown');
    // Walk D6: a zero session spend is "0", never "-0".
    expect(costLabel({ amount: -0, unit: 'CU' })).toBe('0 CU');
    expect(costLabel({ amount: -0.001, unit: 'CU' })).toBe('0 CU');
  });

  test('FR-RGN-20 the normalised error and hint; insufficient_balance has its own message', () => {
    expect(jobErrorText({ code: 'provider_error', hint: 'Try again later.' })).toBe(
      'Provider error. Try again later.',
    );
    expect(jobErrorText({ code: 'x', message: 'the provider refused the mask' })).toBe(
      'The provider refused the mask',
    );
    expect(jobErrorText({ code: 'insufficient_balance' })).toMatch(/Not enough provider balance/);
  });
});

const est = (cost: number | 'unknown', patch: Partial<JobEstimate> = {}): JobEstimate => ({
  estimate_id: 'e1',
  cost: { amount: cost, unit: 'CU' },
  fit: {
    inner: { x: 0, y: 0, w: 64, h: 64 },
    outer: { x: 0, y: 0, w: 64, h: 64 },
    margin: 0,
    sent_w: 64,
    sent_h: 64,
    scale: 1,
  },
  candidate_count: 2,
  spend: 4,
  cap: 20,
  threshold: 5,
  needs: [],
  over_cap: false,
  references: 0,
  reference_notices: [],
  ...patch,
});
const stored = { cap: 20, threshold: 5 };

describe('confirm dialog logic (FR-RGN-14..16, §7)', () => {
  test('FR-RGN-45 the confirm plan carries the references sent ("with N references")', () => {
    const e = est(3, { references: 2 });
    expect(confirmPlan(e, stored, initialForm(e, 5)).references).toBe(2);
    expect(confirmPlan(est(3), stored, initialForm(est(3), 5)).references).toBe(0);
  });

  test('FR-RGN-45 walk R2 "with N references" follows the current selection, not the reused estimate', () => {
    // The estimate saw 2 (one sent once as a duplicate of the other); since then only a
    // non-cost-affecting slot changed, so the estimate is reused.
    const e = est(3, {
      references: 1,
      reference_notices: [
        { code: 'duplicate', message: 'b is a; sent once', key: 'project:b.png' },
      ],
    });
    const plan = (keys: string[] | null) => confirmPlan(e, stored, initialForm(e, 5), keys);
    expect(plan(null).references).toBe(1);
    expect(plan(['project:a.png', 'project:b.png', 'pack:P:face.png']).references).toBe(2);
    expect(plan(['project:a.png', 'pack:P:face.png', 'pack:P:hand.png']).references).toBe(3);
    expect(plan([]).references).toBe(0);
  });

  test('FR-RGN-14 within cap and threshold: Enter confirms, nothing extra is sent', () => {
    const e = est(3);
    const p = confirmPlan(e, stored, initialForm(e, 5));
    expect(p).toMatchObject({
      spendAfter: 7,
      overCap: false,
      overThreshold: false,
      needsSecond: false,
      enterConfirms: true,
      canConfirm: true,
      saveSpend: null,
      confirmations: [],
    });
  });

  test('FR-RGN-14 over the cap: blocked until the cap is raised explicitly, then stored', () => {
    const e = est(3, { spend: 19 });
    const form = initialForm(e, 5);
    const p = confirmPlan(e, stored, form);
    expect(p.overCap).toBe(true);
    expect(p.canConfirm).toBe(false);
    expect(p.blocker).toMatch(/Raise the cap/);
    const raised = confirmPlan(e, stored, { ...form, cap: '30' });
    expect(raised.overCap).toBe(false);
    expect(raised.canConfirm).toBe(true);
    expect(raised.saveSpend).toEqual({ cap: 30, threshold: 5 });
  });

  test('FR-RGN-15 above the threshold: a second explicit confirmation; Enter never confirms', () => {
    const e = est(8, { needs: ['threshold'] });
    const form = initialForm(e, 5);
    const p = confirmPlan(e, stored, form);
    expect(p).toMatchObject({
      overThreshold: true,
      needsSecond: true,
      enterConfirms: false,
      canConfirm: false,
    });
    const second = confirmPlan(e, stored, { ...form, second: true });
    expect(second.canConfirm).toBe(true);
    expect(second.enterConfirms).toBe(false);
    expect(second.confirmations).toEqual(['threshold']);
  });

  test('FR-RGN-15 an unknown cost always needs the second confirmation', () => {
    const e = est('unknown', { needs: ['unknown_cost'] });
    const p = confirmPlan(e, stored, { ...initialForm(e, 5), second: true });
    expect(p).toMatchObject({
      unknownCost: true,
      needsSecond: true,
      enterConfirms: false,
      canConfirm: true,
    });
    expect(p.confirmations).toEqual(['unknown_cost']);
  });

  test('FR-RGN-16 no cap or threshold: the first submit asks for both and stores them', () => {
    const e = est(3, { cap: null, needs: ['set_cap'] });
    const none = { cap: null, threshold: null };
    const form = initialForm(e, null);
    expect(form).toEqual({ cap: '', threshold: '', second: false });
    const p = confirmPlan(e, none, form);
    expect(p.setup).toBe(true);
    expect(p.canConfirm).toBe(false);
    const filled = confirmPlan(e, none, { ...form, cap: '50', threshold: '2' });
    // The new threshold applies to this job: 3 > 2 asks twice.
    expect(filled.overThreshold).toBe(true);
    expect(filled.canConfirm).toBe(false);
    const ok = confirmPlan(e, none, { cap: '50', threshold: '2', second: true });
    expect(ok.canConfirm).toBe(true);
    expect(ok.saveSpend).toEqual({ cap: 50, threshold: 2 });
    expect(ok.confirmations).toEqual(['set_cap', 'threshold']);
  });
});

const row = (
  job_id: string,
  n: number | null,
  state: RgnIteration['state'],
  accepted = false,
): RgnIteration => ({
  job_id,
  n,
  dir: n === null ? null : `results/${n} x`,
  slug: n === null ? null : 'x',
  state,
  kept: 1,
  returned: 2,
  cost: { amount: 2, unit: 'CU' },
  accepted_in_stack: accepted,
  references: 0,
  reference_files: [],
});
const cand = (
  id: string,
  job_id: string,
  index: number,
  patch: Partial<CandidateRecord> = {},
): CandidateRecord => ({
  id,
  job_id,
  index,
  file: `results/001 x/${index + 1}.png`,
  status: 'new',
  starred: false,
  trash_id: null,
  ...patch,
});

describe('iteration tracker logic', () => {
  test('FR-RGN-40 newest first, the active job without candidates leads; the newest is selected by default', () => {
    const rows = [row('J1', 1, 'closed', true), row('J3', null, 'running'), row('J2', 2, 'closed')];
    expect(orderRows(rows).map((r) => r.job_id)).toEqual(['J3', 'J2', 'J1']);
    expect(defaultSelection(rows, null)).toBe('J3');
    expect(defaultSelection(rows, 'J1')).toBe('J1');
    expect(defaultSelection(rows, 'gone')).toBe('J3');
    expect(stripJobId(rows)).toBe('J3');
    expect(stripJobId([row('J1', 1, 'closed', true)])).toBeNull();
  });

  test('FR-RGN-28 walk D6 a job closed by the next submit shows Closed, not its last live state', () => {
    const live = { state: 'fetched' as const };
    expect(shownState(row('J1', 1, 'closed'), live)).toBe('closed');
    expect(shownState(row('J1', 1, 'failed'), { state: 'running' })).toBe('failed');
    // A live event still leads while the core's row isn't final yet.
    expect(shownState(row('J2', null, 'submitted'), { state: 'running' })).toBe('running');
    expect(shownState(row('J2', 2, 'fetched'), undefined)).toBe('fetched');
  });

  test('FR-RGN-21 R62 Retry on the newest job in any final state, closed included; never an older one', () => {
    const retryIds = (rows: RgnIteration[]) =>
      rows.filter((r) => offersRetry(rows, r)).map((r) => r.job_id);
    expect(retryIds([row('J1', 1, 'closed', true), row('J2', 2, 'closed')])).toEqual(['J2']);
    for (const st of ['failed', 'canceled', 'lost', 'fetched'] as const)
      expect(retryIds([row('J1', 1, 'closed'), row('J2', 2, st)])).toEqual(['J2']);
    expect(retryIds([row('J1', 1, 'closed'), row('J3', null, 'running')])).toEqual([]);
    expect(retryIds([])).toEqual([]);
  });

  test('R84 FR-GEN-32 every final generate iteration offers Retry; region edit rows stay newest-only', () => {
    const gen = (r: RgnIteration): RgnIteration => ({ ...r, operation: 'generate' });
    const rows = [
      gen(row('G1', 1, 'closed', true)),
      row('E2', 2, 'closed'),
      gen(row('G3', 3, 'failed')),
      row('E4', 4, 'closed'),
      gen(row('G5', null, 'running')),
    ];
    expect(rows.filter((r) => offersRetry(rows, r)).map((r) => r.job_id)).toEqual(['G1', 'G3']);
  });

  test('FR-RGN-40 the picker gets the iteration’s non-trashed candidates in provider order', () => {
    const all = [
      cand('B', 'J1', 1, { starred: true, status: 'dismissed' }),
      cand('A', 'J1', 0, { status: 'accepted' }),
      cand('C', 'J1', 2, { status: 'dismissed', trash_id: 'T1' }),
      cand('D', 'J1', 3, { status: 'invalid' }),
      cand('E', 'J2', 0),
      cand('F', 'J1', 4, { aspect_warning: true }),
    ];
    const ok = { starred: false, accepted: false, aspectWarning: false };
    expect(iterationCandidates(all, 'J1')).toEqual([
      { ...ok, id: 'A', index: 1, status: 'ready', accepted: true },
      { ...ok, id: 'B', index: 2, status: 'ready', starred: true },
      { ...ok, id: 'D', index: 4, status: 'invalid' },
      // FR-RGN-24: the core's aspect warning reaches the picker.
      { ...ok, id: 'F', index: 5, status: 'ready', aspectWarning: true },
    ]);
  });

  test('FR-RGN-19/21/28 job actions per state', () => {
    expect(['submitted', 'queued', 'running'].every((s) => canCancel(s as never))).toBe(true);
    expect(canCancel('unknown')).toBe(false);
    expect(canCancel('fetched')).toBe(false);
    expect(canRetry('failed') && canRetry('fetched') && canRetry('lost')).toBe(true);
    expect(canRetry('running')).toBe(false);
    expect(canDismiss('fetched') && canDismiss('failed')).toBe(true);
    expect(canDismiss('closed') || canDismiss('running')).toBe(false);
  });

  test('FR-RGN-45 the refs chip tooltip lists each file, project or its pack, and role', () => {
    const r: RgnIteration = {
      ...row('J1', 1, 'fetched'),
      references: 2,
      reference_files: [
        { file: 'mood.png', pack_name: null, role: 'style' },
        { file: 'sword.png', pack_name: 'Blades', role: null },
      ],
    };
    expect(refsTooltip(r)).toBe('mood.png · project · style\nsword.png · Blades');
    expect(refsTooltip(row('J2', 2, 'fetched'))).toBe('');
  });

  test('owner #56 the picker keeps dismissed results; only discarded ones leave it', () => {
    const all = [
      cand('A', 'J1', 0, { status: 'accepted' }),
      cand('B', 'J1', 1, { status: 'dismissed' }),
      cand('C', 'J1', 2, { status: 'discarded', trash_id: 'T1' }),
    ];
    expect(iterationCandidates(all, 'J1').map((c) => c.id)).toEqual(['A', 'B']);
  });
});

test('walk-gen L4 FR-GEN-28 the patches-kept notice agrees with its count', () => {
  expect(PATCHES_KEPT(1)).toBe('1 patch layer kept in history (undo restores it).');
  expect(PATCHES_KEPT(3)).toBe('3 patch layers kept in history (undo restores them).');
});

test('R84 FR-RGN-45 FR-GEN-21 the confirm names what is sent besides the prompt', () => {
  expect(sentWith(2, false)).toBe('with 2 references');
  expect(sentWith(1, true)).toBe('with a sketch and 1 reference');
  expect(sentWith(0, true)).toBe('with a sketch');
});
