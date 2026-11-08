// One capability control (RGN Appendix A): slider + number, number, select, toggle or text,
// with its range, step, effective allowed values and required mark from `evaluate` (FR-RGN-33).
import { useEffect, useState, type KeyboardEvent } from 'react';
import { Select, Slider, Switch } from 'radix-ui';

import type { ControlState } from '../../api';
import { controlKind, snap, type Control } from './controls';

const NONE = '__none__';
const keyOf = (v: unknown) => JSON.stringify(v);

function NumberInput({
  id,
  c,
  value,
  onCommit,
  label,
}: {
  id?: string;
  c: Control;
  value: number | null;
  onCommit: (v: number | null) => void;
  label: string;
}) {
  const [text, setText] = useState(value === null ? '' : String(value));
  useEffect(() => setText(value === null ? '' : String(value)), [value]);
  const commit = () => {
    if (text.trim() === '') return onCommit(c.required ? value : null);
    const n = Number(text);
    if (!Number.isFinite(n)) return setText(value === null ? '' : String(value));
    const v = snap(c, n);
    setText(String(v));
    if (v !== value) onCommit(v);
  };
  return (
    <input
      type="number"
      id={id}
      className="rgn-number tabular"
      aria-label={label}
      value={text}
      min={c.min ?? undefined}
      max={c.max ?? undefined}
      step={c.step ?? (c.type === 'int' ? 1 : 'any')}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
        if (e.key === 'Enter') commit();
      }}
    />
  );
}

function SliderNumber({
  id,
  c,
  value,
  onCommit,
  label,
}: {
  id: string;
  c: Control;
  value: number | null;
  onCommit: (v: number) => void;
  label: string;
}) {
  const min = c.min ?? 0;
  const max = c.max ?? 1;
  const [drag, setDrag] = useState<number | null>(null);
  const shown = drag ?? value ?? min;
  return (
    <div className="rgn-slider-row">
      <Slider.Root
        className="rgn-slider"
        min={min}
        max={max}
        step={c.step ?? (c.type === 'int' ? 1 : (max - min) / 100)}
        value={[shown]}
        onValueChange={([v]) => setDrag(v ?? null)}
        onValueCommit={([v]) => {
          setDrag(null);
          if (v !== undefined) onCommit(snap(c, v));
        }}
      >
        <Slider.Track className="rgn-slider-track">
          <Slider.Range className="rgn-slider-range" />
        </Slider.Track>
        <Slider.Thumb className="rgn-slider-thumb" aria-label={label} />
      </Slider.Root>
      <NumberInput
        id={id}
        c={c}
        value={drag ?? value}
        onCommit={(v) => v !== null && onCommit(v)}
        label={label}
      />
    </div>
  );
}

export function ControlField({
  c,
  value,
  state,
  onChange,
}: {
  c: Control;
  value: unknown;
  state: ControlState | undefined;
  onChange: (v: unknown) => void;
}) {
  const kind = controlKind(c);
  const required = state?.required ?? c.required;
  const label = c.name.replace(/_/g, ' ');
  const allowed = state && state.allowed.length ? state.allowed : c.allowed;
  const id = `rgn-control-${c.name}`;

  let field: React.ReactNode = null;
  switch (kind) {
    case 'slider':
      field = (
        <SliderNumber
          id={id}
          c={c}
          value={typeof value === 'number' ? value : null}
          onCommit={onChange}
          label={label}
        />
      );
      break;
    case 'number':
      field = (
        <NumberInput
          id={id}
          c={c}
          value={typeof value === 'number' ? value : null}
          onCommit={onChange}
          label={label}
        />
      );
      break;
    case 'select':
      field = (
        <Select.Root
          value={value === null || value === undefined ? NONE : keyOf(value)}
          onValueChange={(k) => onChange(k === NONE ? null : allowed.find((a) => keyOf(a) === k))}
        >
          <Select.Trigger className="rgn-select" id={id} aria-label={label}>
            <Select.Value placeholder="—" />
            <Select.Icon className="rgn-select-icon">▾</Select.Icon>
          </Select.Trigger>
          <Select.Portal>
            <Select.Content className="rgn-select-content" position="popper" sideOffset={2}>
              <Select.Viewport>
                {!required && (
                  <Select.Item className="rgn-select-item" value={NONE}>
                    <Select.ItemText>—</Select.ItemText>
                  </Select.Item>
                )}
                {allowed.map((a) => (
                  <Select.Item
                    key={keyOf(a)}
                    className="rgn-select-item"
                    value={keyOf(a)}
                    data-testid="rgn-control-option"
                  >
                    <Select.ItemText>{String(a)}</Select.ItemText>
                  </Select.Item>
                ))}
              </Select.Viewport>
            </Select.Content>
          </Select.Portal>
        </Select.Root>
      );
      break;
    case 'toggle':
      field = (
        <Switch.Root
          className="rgn-switch"
          id={id}
          aria-label={label}
          checked={value === true}
          onCheckedChange={(v) => onChange(v)}
        >
          <Switch.Thumb className="rgn-switch-thumb" />
        </Switch.Root>
      );
      break;
    case 'text':
      field = (
        <TextInput
          id={id}
          label={label}
          value={typeof value === 'string' ? value : ''}
          onCommit={onChange}
        />
      );
      break;
    default:
      return null;
  }

  return (
    <div
      className="rgn-control"
      data-testid="rgn-control-input"
      data-record-id={c.name}
      data-kind={kind}
      data-required={required ? 'true' : 'false'}
    >
      <label className="rgn-control-label" htmlFor={id}>
        {label}
        {required && (
          <span className="rgn-required" title="Required">
            *
          </span>
        )}
        {c.cost_affecting && (
          <span className="rgn-costly" title="Changes the cost">
            ¤
          </span>
        )}
      </label>
      {field}
      {state?.error && state.error.code !== 'required' && (
        <div className="error small">{state.error.message}</div>
      )}
    </div>
  );
}

function TextInput({
  id,
  label,
  value,
  onCommit,
}: {
  id: string;
  label: string;
  value: string;
  onCommit: (v: string) => void;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <input
      type="text"
      id={id}
      aria-label={label}
      className="rgn-text"
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => text !== value && onCommit(text)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' && text !== value) onCommit(text);
      }}
    />
  );
}
