// Capability-driven controls (RGN Appendix A, #37): which UI control each capability control
// gets, the values sent (FR-RGN-34), resets on remembered or narrowed values (FR-RGN-11/33) and
// the cost signature that drives the estimate (FR-RGN-12). Pure; no core calls.
import type { Capabilities, ControlState, ControlsEvaluation } from '../../api';

export type Control = Capabilities['controls'][number];
export type Values = Record<string, unknown>;

/** Appendix A. `none` = never shown (source, mask, reference images). The prompt and seed are
 *  rendered in their own sections, bound by role (R28). */
export type ControlKind =
  'none' | 'prompt' | 'seed' | 'slider' | 'number' | 'select' | 'toggle' | 'text';

export function controlKind(c: Control): ControlKind {
  if (c.type === 'image' || c.type === 'mask' || c.role === 'source' || c.role === 'mask')
    return 'none';
  if (c.role === 'reference') return 'none';
  if (c.role === 'prompt' && c.type === 'string') return 'prompt';
  if (c.type === 'seed' || c.role === 'seed') return 'seed';
  switch (c.type) {
    case 'int':
    case 'float':
      return c.min !== null && c.max !== null ? 'slider' : 'number';
    case 'enum':
      return 'select';
    case 'bool':
      return 'toggle';
    case 'string':
      return 'text';
    default:
      return 'none';
  }
}

/** Controls the panel holds a value for (everything but images, prompt and seed). */
export const isValueControl = (c: Control): boolean =>
  !['none', 'prompt', 'seed'].includes(controlKind(c));

export const promptControl = (caps: Capabilities): Control | undefined =>
  caps.controls.find((c) => controlKind(c) === 'prompt');
export const seedControl = (caps: Capabilities): Control | undefined =>
  caps.controls.find((c) => controlKind(c) === 'seed');

/** Defaults prefill every control (Appendix A). */
export function defaultsOf(caps: Capabilities): Values {
  const v: Values = {};
  for (const c of caps.controls) if (isValueControl(c)) v[c.name] = c.default ?? null;
  return v;
}

const same = (a: unknown, b: unknown) => a === b;

/** Static check of a value against its capability control (type, range, step, allowed). */
export function fitsControl(c: Control, v: unknown, allowed: unknown[] = c.allowed): boolean {
  if (v === null || v === undefined) return true;
  switch (c.type) {
    case 'int':
    case 'float': {
      if (typeof v !== 'number' || !Number.isFinite(v)) return false;
      if (c.type === 'int' && !Number.isInteger(v)) return false;
      if ((c.min !== null && v < c.min) || (c.max !== null && v > c.max)) return false;
      break;
    }
    case 'bool':
      if (typeof v !== 'boolean') return false;
      break;
    case 'string':
      if (typeof v !== 'string') return false;
      break;
    case 'enum':
      break;
    default:
      return false;
  }
  return allowed.length === 0 || allowed.some((a) => same(a, v));
}

export interface Restored {
  values: Values;
  touched: string[];
  notices: string[];
}

/** FR-RGN-11: the defaults with the remembered values over them. The core's evaluation then
 *  resets the ones the capabilities reject and drops unknown names (`fromEvaluation`). */
export function withRemembered(
  caps: Capabilities,
  remembered: Values | null | undefined,
): Restored {
  const values = { ...defaultsOf(caps), ...(remembered ?? {}) };
  return { values, touched: Object.keys(remembered ?? {}), notices: [] };
}

/** FR-RGN-11/33: applies `rgn_controls_evaluate`: a reset value (outside the effective allowed
 *  set, wrong type, out of range) takes the core's value with a notice; names this model has no
 *  control for are dropped with a notice. */
export function fromEvaluation(
  caps: Capabilities,
  values: Values,
  touched: string[],
  ev: ControlsEvaluation,
): Restored {
  const next = { ...values };
  let t = [...touched];
  const notices: string[] = [];
  for (const name of ev.unknown) {
    if (!(name in next)) continue;
    delete next[name];
    t = t.filter((n) => n !== name);
    notices.push(`${name}: not a setting of this model; dropped.`);
  }
  for (const c of ev.controls) {
    const ctl = caps.controls.find((x) => x.name === c.name);
    if (!ctl || !isValueControl(ctl) || !c.visible || !c.reset) continue;
    const old = next[c.name];
    next[c.name] = c.value ?? null;
    t = t.filter((n) => n !== c.name);
    notices.push(
      `${c.name}: “${String(old)}” isn’t available with these settings; reset to the default.`,
    );
  }
  return { values: next, touched: t, notices };
}

export const stateOf = (evaluation: ControlState[] | null, name: string) =>
  evaluation?.find((e) => e.name === name);

/** Values passed to `rgn_controls_evaluate`: the control values plus the prompt and a pinned
 *  seed under their control names (PRV evaluate takes every value by name). */
export function evaluateInput(
  caps: Capabilities,
  values: Values,
  prompt: string,
  seed: number | null,
): Values {
  const v: Values = { ...values };
  const p = promptControl(caps);
  if (p) v[p.name] = prompt;
  const s = seedControl(caps);
  if (s && seed !== null) v[s.name] = seed;
  return v;
}

/** FR-RGN-33: visible required controls without a value (the prompt by FR-RGN-09). */
export function missingRequired(
  caps: Capabilities,
  evaluation: ControlState[] | null,
  values: Values,
  prompt: string,
): string[] {
  const out: string[] = [];
  if (caps.prompt_required && prompt.trim() === '') out.push('prompt');
  for (const c of caps.controls) {
    if (!isValueControl(c)) continue;
    const e = stateOf(evaluation, c.name);
    if (e && !e.visible) continue;
    const required = e?.required ?? c.required;
    const v = values[c.name];
    if (required && (v === null || v === undefined || v === '')) out.push(c.name);
  }
  return out;
}

/** FR-RGN-34: visible controls that are required or changed by the user; untouched optional
 *  ones are omitted so the provider applies its default. Hidden ones are never sent (33). */
export function sentValues(
  caps: Capabilities,
  evaluation: ControlState[] | null,
  values: Values,
  touched: string[],
): Values {
  const out: Values = {};
  for (const c of caps.controls) {
    if (!isValueControl(c)) continue;
    const e = stateOf(evaluation, c.name);
    if (e && !e.visible) continue;
    const v = values[c.name];
    if (v === null || v === undefined) continue;
    if ((e?.required ?? c.required) || touched.includes(c.name)) out[c.name] = v;
  }
  return out;
}

/** FR-RGN-12: the inputs that change the cost: model, sent size, cost-affecting controls. */
export function costSignature(
  modelId: string,
  sent: { w: number; h: number },
  caps: Capabilities,
  sentControls: Values,
): string {
  const costly = caps.controls
    .filter((c) => c.cost_affecting)
    .map((c) => `${c.name}=${JSON.stringify(sentControls[c.name] ?? null)}`);
  return `${modelId}|${sent.w}x${sent.h}|${costly.join(',')}`;
}

/** Snaps a numeric value to the control's step from its min (Appendix A "step snapping"). */
export function snap(c: Control, v: number): number {
  let x = v;
  if (c.step && c.step > 0) {
    const base = c.min ?? 0;
    x = base + Math.round((x - base) / c.step) * c.step;
    x = Number(x.toFixed(6));
  }
  if (c.min !== null) x = Math.max(c.min, x);
  if (c.max !== null) x = Math.min(c.max, x);
  if (c.type === 'int') x = Math.round(x);
  return x;
}
