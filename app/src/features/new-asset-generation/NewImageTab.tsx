// The New image tab (new-asset-generation §7), top to bottom: Model (picker, capability controls,
// warnings) · Size · Structural sketch (hidden when the model takes none) · References (REF's
// block) · Prompt (PRM's block, FR-GEN-16) · Seed · Cost · Generate. Rendered by RGN's Generate
// section as the "New image" tab (OQ-GEN-09); only mounted while it is the visible tab, so it is
// the prompt chat's host then (FR-GEN-36): it publishes its draft and clears it when unmounted.
import { useEffect, useState } from 'react';
import { RadioGroup, Select, Switch } from 'radix-ui';

import { pickPaths } from '../../api';
import { chatContext } from '../../shared/chatContext';
import { useProjectState } from '../../store/project';
import { PromptBlock } from '../prompt-helper';
import { openProviderSettings } from '../provider';
import { ReferencesBlock } from '../references';
import {
  amountLabel,
  ControlField,
  costLabel,
  errorText,
  isValueControl,
  SpendConfirm,
  stateOf,
  useActiveJobId,
} from '../region-edit';
import {
  CELLS,
  confirmSize,
  isRatio,
  longSideLimits,
  PPS,
  ratioOptions,
  resultLabel,
} from './size';
import {
  costUnit,
  currentDraft,
  gen,
  isOffline,
  singleResult,
  submitBlocker,
  takesSketch,
  useGen,
  type GenState,
} from './store';
import './gen.css';

/** Binds the store to the open project and reports the tab as shown (FR-GEN-36). */
function useGenSession(): void {
  const { project } = useProjectState();
  const pid = project?.project_id ?? null;
  useEffect(() => {
    if (project) void gen.attach(project);
    else gen.detach();
    // Re-attach only per project / read-only change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pid, project?.read_only]);
  useEffect(() => {
    gen.setVisible(true);
    return () => gen.setVisible(false);
  }, []);
  useChatHost();
}

/** FR-GEN-36 / FR-PRM-23: while mounted (= the visible tab) the tab is the chat's host: it
 *  publishes the draft its estimate takes (debounced by the seam) and clears it when it is
 *  hidden or unmounted (a tab switch or closing the project). */
function useChatHost(): void {
  const input = useGen((x) => {
    const d = currentDraft(x);
    return x.projectId && d ? JSON.stringify(d) : null;
  });
  useEffect(() => {
    if (input) chatContext.publish('gen', JSON.parse(input));
  }, [input]);
  useEffect(() => () => chatContext.clear('gen'), []);
}

/** An integer field that commits on Enter or blur (out-of-range values are clamped by the store). */
function IntField(props: {
  id: string;
  testId: string;
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  onCommit(v: number): void;
}) {
  const [text, setText] = useState(String(props.value));
  useEffect(() => setText(String(props.value)), [props.value]);
  const commit = () => {
    const n = Number(text);
    if (text.trim() !== '' && Number.isFinite(n) && n !== props.value) props.onCommit(n);
    else setText(String(props.value));
  };
  return (
    <input
      id={props.id}
      type="number"
      className="rgn-number tabular"
      aria-label={props.label}
      data-testid={props.testId}
      min={props.min}
      max={props.max}
      step={props.step ?? 1}
      disabled={props.disabled}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === 'Enter' && commit()}
    />
  );
}

function ModelSection({ s }: { s: GenState }) {
  const warnings = [...(s.caps?.warnings ?? []), ...s.notices];
  return (
    <div className="rgn-section">
      <label className="rgn-control-label" htmlFor="gen-model">
        Model
        {s.modelsStale && (
          <span className="pf-chip" data-testid="gen-panel-stale" title="Offline: cached list">
            cached
          </span>
        )}
      </label>
      <Select.Root value={s.modelId ?? ''} onValueChange={(id) => void gen.pickModel(id)}>
        <Select.Trigger
          className="rgn-select"
          id="gen-model"
          aria-label="Model"
          data-testid="gen-panel-model"
          disabled={s.models.length === 0}
        >
          <Select.Value placeholder={s.modelsLoading ? 'Loading models…' : 'Pick a model'} />
          <Select.Icon className="rgn-select-icon">▾</Select.Icon>
        </Select.Trigger>
        <Select.Portal>
          <Select.Content className="rgn-select-content" position="popper" sideOffset={2}>
            <Select.Viewport>
              {s.models.map((m) => (
                <Select.Item
                  key={m.id}
                  className="rgn-select-item"
                  value={m.id}
                  data-testid="gen-panel-model-option"
                  data-record-id={m.id}
                >
                  <Select.ItemText>{m.name}</Select.ItemText>
                </Select.Item>
              ))}
            </Select.Viewport>
          </Select.Content>
        </Select.Portal>
      </Select.Root>
      {s.modelsError && (
        <p className="error small">{errorText(s.modelsError.code, s.modelsError.message)}</p>
      )}
      {s.capsLoading && (
        <p className="muted small" data-testid="gen-panel-loading">
          Loading capabilities…
        </p>
      )}
      {s.capsError && (
        <p className="error small">{errorText(s.capsError.code, s.capsError.message)}</p>
      )}
      {s.caps && (
        <div className="rgn-controls" data-testid="gen-panel-controls">
          {s.caps.controls
            // FR-GEN-11: PRV never lists the inputs behind `output_size`; the Size block sets them.
            .filter((c) => isValueControl(c))
            .filter((c) => stateOf(s.evaluation, c.name)?.visible ?? true)
            .map((c) => (
              <ControlField
                key={c.name}
                prefix="gen"
                c={c}
                value={s.values[c.name]}
                state={stateOf(s.evaluation, c.name)}
                onChange={(v) => gen.setValue(c.name, v)}
              />
            ))}
          {singleResult(s.caps) && (
            <p className="muted small" data-testid="gen-panel-count-single">
              1 result per job (this model)
            </p>
          )}
        </div>
      )}
      {warnings.length > 0 && (
        <div className="rgn-warning" data-testid="gen-panel-warning" role="status">
          <ul>
            {warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
          {s.notices.length > 0 && (
            <button type="button" className="link small" onClick={() => gen.dismissNotices()}>
              Dismiss notices
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** FR-GEN-07..10: a grid for Maps, a ratio (and a long side on wh models) for Tokens and Items. */
function SizeSection({ s }: { s: GenState }) {
  const size = s.size;
  const notices = s.fit?.notices ?? [];
  return (
    <div className="rgn-section" data-testid="gen-size" data-mode={size.mode}>
      <span className="rgn-control-label">Size</span>
      {size.mode === 'grid' ? (
        <div className="gen-grid">
          <label className="gen-field">
            <span className="muted small">Columns</span>
            <IntField
              id="gen-size-cols"
              testId="gen-size-cols"
              label="Columns"
              value={size.cols}
              min={CELLS.min}
              max={CELLS.max}
              onCommit={(cols) => gen.setGrid({ cols })}
            />
          </label>
          <span className="muted gen-times">×</span>
          <label className="gen-field">
            <span className="muted small">Rows</span>
            <IntField
              id="gen-size-rows"
              testId="gen-size-rows"
              label="Rows"
              value={size.rows}
              min={CELLS.min}
              max={CELLS.max}
              onCommit={(rows) => gen.setGrid({ rows })}
            />
          </label>
          {s.caps?.output_size?.mode === 'aspect' ? (
            // FR-GEN-09: an aspect model sets the size by ratio; px per square follows the result.
            <span className="muted small gen-field-note" data-testid="gen-size-pps-note">
              px per square: set by the model’s ratio, shown once a result is in
            </span>
          ) : (
            <label className="gen-field">
              <span className="muted small">px per square</span>
              <IntField
                id="gen-size-pps"
                testId="gen-size-pps"
                label="Pixels per square"
                value={size.pps}
                min={PPS.min}
                max={PPS.max}
                onCommit={(pps) => gen.setGrid({ pps })}
              />
            </label>
          )}
        </div>
      ) : (
        <RatioFields s={s} aspect={size.aspect} longSide={size.long_side} />
      )}
      {s.fit && (
        <div className="muted small tabular" data-testid="gen-size-result">
          sent {resultLabel(size, s.fit, s.lastResult)}
        </div>
      )}
      {s.fitError && (
        <p className="error small" data-testid="gen-size-error">
          {errorText(s.fitError.code, s.fitError.message)}
        </p>
      )}
      {notices.length > 0 && (
        <ul className="rgn-warning small" data-testid="gen-size-notices" role="status">
          {notices.map((n) => (
            <li key={n.code} data-code={n.code}>
              {n.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function RatioFields({ s, aspect, longSide }: { s: GenState; aspect: string; longSide: number }) {
  const options = ratioOptions(s.caps);
  const lim = longSideLimits(s.caps);
  // A saved ratio this model lacks still shows, so the select never goes blank.
  const shown = options.includes(aspect) || !isRatio(aspect) ? options : [aspect, ...options];
  return (
    <div className="gen-grid">
      <label className="gen-field">
        <span className="muted small">Aspect</span>
        <Select.Root value={aspect} onValueChange={(v) => gen.setRatio({ aspect: v })}>
          <Select.Trigger className="rgn-select" aria-label="Aspect" data-testid="gen-size-aspect">
            <Select.Value />
            <Select.Icon className="rgn-select-icon">▾</Select.Icon>
          </Select.Trigger>
          <Select.Portal>
            <Select.Content className="rgn-select-content" position="popper" sideOffset={2}>
              <Select.Viewport>
                {shown.map((r) => (
                  <Select.Item
                    key={r}
                    className="rgn-select-item"
                    value={r}
                    data-testid="gen-size-aspect-option"
                    data-record-id={r}
                  >
                    <Select.ItemText>{r}</Select.ItemText>
                  </Select.Item>
                ))}
              </Select.Viewport>
            </Select.Content>
          </Select.Portal>
        </Select.Root>
      </label>
      {lim && (
        <label className="gen-field">
          <span className="muted small">Long side (px)</span>
          <IntField
            id="gen-size-long-side"
            testId="gen-size-long-side"
            label="Long side"
            value={longSide}
            min={lim.min}
            max={lim.max}
            step={lim.step}
            onCommit={(long_side) => gen.setRatio({ long_side })}
          />
        </label>
      )}
    </div>
  );
}

/** FR-GEN-14: one `structure/` image or None (the default), Add…, badges. */
function SketchSection({ s }: { s: GenState }) {
  const files = s.sketches ?? [];
  if (!takesSketch(s)) {
    return files.length > 0 ? (
      <p className="muted small" data-testid="gen-sketch-unsupported">
        This model takes no structural sketch
      </p>
    ) : null;
  }
  const chosenMissing = s.sketch !== null && !files.some((f) => f.name === s.sketch);
  return (
    <div className="rgn-section" data-testid="gen-sketch">
      <div className="rgn-row">
        <span className="rgn-control-label">Structural sketch</span>
        <AddSketch disabled={!!s.readOnly} />
      </div>
      <RadioGroup.Root
        className="gen-sketch-list"
        aria-label="Structural sketch"
        value={s.sketch ?? ''}
        disabled={!!s.readOnly}
        onValueChange={(v) => gen.setSketch(v === '' ? null : v)}
      >
        <SketchRow value="" testId="gen-sketch-none" label="None" />
        {files.map((f) => (
          <SketchRow
            key={f.name}
            value={f.name}
            testId="gen-sketch-item"
            label={f.name}
            thumb={f.thumb_url ?? null}
            badge={f.state === 'cloud_only' ? 'cloud' : f.state === 'corrupt' ? 'corrupt' : null}
            disabled={f.state === 'corrupt'}
            flag={f.name === s.sketch ? s.sketchFlag : null}
          />
        ))}
        {chosenMissing && (
          <SketchRow
            value={s.sketch!}
            testId="gen-sketch-item"
            label={s.sketch!}
            badge="missing"
            flag={s.sketchFlag}
            disabled
          />
        )}
      </RadioGroup.Root>
      {files.length === 0 && (
        <p className="muted small">Add a rough layout image to guide the map’s structure.</p>
      )}
      {s.sketchError && (
        <p className="error small" data-testid="gen-sketch-error">
          {errorText(s.sketchError.code, s.sketchError.message)}
        </p>
      )}
    </div>
  );
}

function SketchRow(props: {
  value: string;
  testId: string;
  label: string;
  thumb?: string | null;
  badge?: string | null;
  disabled?: boolean;
  flag?: GenState['sketchFlag'];
}) {
  const flag = props.flag?.changed_since
    ? `changed since ${props.flag.changed_since}`
    : props.flag?.missing && props.badge !== 'missing'
      ? 'missing'
      : null;
  return (
    <label className="gen-sketch-row" data-disabled={props.disabled ? 'true' : undefined}>
      <RadioGroup.Item
        className="gen-radio"
        value={props.value}
        disabled={props.disabled}
        data-testid={props.testId}
        data-record-id={props.value || undefined}
        aria-label={props.label}
      >
        <RadioGroup.Indicator className="gen-radio-dot" />
      </RadioGroup.Item>
      {props.thumb ? (
        <img className="gen-sketch-thumb" src={props.thumb} alt="" />
      ) : (
        props.value && <span className="gen-sketch-thumb" aria-hidden="true" />
      )}
      <span className="gen-sketch-name">{props.label}</span>
      {props.badge && (
        <span className="pf-chip" data-testid="gen-sketch-badge" data-badge={props.badge}>
          {props.badge}
        </span>
      )}
      {flag && (
        <span className="pf-chip warning" data-testid="gen-sketch-flag">
          {flag}
        </span>
      )}
    </label>
  );
}

/** Add… (#53): the native image dialog; where none is available (tests on the mocked core), a
 *  field takes paths, one per line. */
function AddSketch({ disabled }: { disabled: boolean }) {
  const [typing, setTyping] = useState(false);
  const [text, setText] = useState('');
  const paths = text
    .split(/\r?\n/)
    .map((l) => l.trim().replace(/^"(.*)"$/, '$1'))
    .filter(Boolean);
  if (typing)
    return (
      <form
        className="gen-add-form"
        onSubmit={(e) => {
          e.preventDefault();
          setTyping(false);
          setText('');
          void gen.addSketches(paths);
        }}
      >
        <textarea
          aria-label="Image paths, one per line"
          data-testid="gen-sketch-add-paths"
          rows={2}
          value={text}
          autoFocus
          onChange={(e) => setText(e.target.value)}
        />
        <button type="submit" data-testid="gen-sketch-add-ok" disabled={paths.length === 0}>
          Add
        </button>
        <button type="button" className="link small" onClick={() => setTyping(false)}>
          Cancel
        </button>
      </form>
    );
  return (
    <button
      type="button"
      className="small"
      data-testid="gen-sketch-add"
      disabled={disabled}
      onClick={() =>
        pickPaths({ title: 'Add structural sketches', multiple: true, images: true }).then(
          (picked) => void gen.addSketches(picked),
          () => setTyping(true),
        )
      }
    >
      Add…
    </button>
  );
}

/** FR-GEN-16: PRM's prompt block (typed prompt, helpers, "Final prompt (as sent)", chat): the
 *  final prompt is the typed prompt plus the ticked helpers, never the blend instruction. */
function PromptSection({ s }: { s: GenState }) {
  const seedCtl = s.caps?.controls.find((c) => c.type === 'seed' || c.role === 'seed');
  return (
    <div className="rgn-section">
      <PromptBlock
        host="gen"
        kind={s.kind}
        prompt={s.prompt}
        onPromptChange={(t) => gen.setPrompt(t)}
        helperIds={s.promptHelpers}
        onHelperIdsChange={(ids) => gen.setPromptHelpers(ids)}
        readOnly={!!s.readOnly}
        required={!!s.caps?.prompt_required}
        retry={s.promptRetry}
      />
      {s.caps?.seed && seedCtl && (
        <div className="rgn-row" data-testid="gen-panel-seed">
          <span className="muted">Seed</span>
          <label className="rgn-check">
            <Switch.Root
              className="rgn-switch"
              aria-label="Pin the seed"
              data-testid="gen-panel-seed-pin"
              checked={s.seedMode === 'pinned'}
              onCheckedChange={(v) => gen.setSeedMode(v ? 'pinned' : 'random')}
            >
              <Switch.Thumb className="rgn-switch-thumb" />
            </Switch.Root>
            {s.seedMode === 'pinned' ? 'pinned' : 'random'}
          </label>
          {s.seedMode === 'pinned' && (
            <input
              type="number"
              min={0}
              className="rgn-number tabular"
              aria-label="Seed"
              data-testid="gen-panel-seed-value"
              value={s.seed ?? 0}
              onChange={(e) =>
                gen.setSeed(Number.isFinite(Number(e.target.value)) ? Number(e.target.value) : null)
              }
            />
          )}
        </div>
      )}
    </div>
  );
}

function CostSection({ s }: { s: GenState }) {
  const e = s.estimate;
  const unit = costUnit(s);
  let line: React.ReactNode;
  if (s.readOnly) line = <span className="muted">No estimate: the project is read-only.</span>;
  else if (isOffline(s)) line = <span className="muted">No estimate while offline.</span>;
  else if (e.status === 'waiting' || e.status === 'loading')
    line = <span className="muted">Estimating…</span>;
  else if (e.status === 'ready' && e.result)
    line = (
      <span className={e.result.cost.amount === 'unknown' ? 'warning' : ''}>
        Estimate: <strong className="tabular">{costLabel(e.result.cost)}</strong>
      </span>
    );
  else if (e.status === 'error' && e.error)
    line = <span className="error">{errorText(e.error.code, e.error.message)}</span>;
  else line = <span className="muted">Estimate: —</span>;
  return (
    <div className="rgn-section">
      <div
        className="rgn-estimate"
        data-testid="gen-panel-estimate"
        data-status={s.readOnly || isOffline(s) ? 'idle' : e.status}
        data-cost={e.result ? String(e.result.cost.amount) : ''}
      >
        {line}
      </div>
      {s.spend && (
        <div className="muted small tabular" data-testid="gen-panel-spend">
          Session {amountLabel(s.spend.spend, unit)}
          {s.spend.cap !== null ? ` of ${amountLabel(s.spend.cap, unit)} cap` : ' · no cap set yet'}
        </div>
      )}
    </div>
  );
}

function SubmitSection({ s }: { s: GenState }) {
  const activeJob = useActiveJobId();
  const blocker = submitBlocker(s, activeJob);
  const notConfigured = s.provider && !s.provider.configured;
  const authError = s.provider?.connectivity === 'auth_error' || s.estimate.error?.code === 'auth';
  return (
    <div className="rgn-section">
      {notConfigured || authError ? (
        <button
          type="button"
          className="primary rgn-submit"
          data-testid="gen-panel-setup"
          onClick={openProviderSettings}
        >
          Set up provider
        </button>
      ) : (
        <button
          type="button"
          className="primary rgn-submit"
          data-testid="gen-panel-submit"
          disabled={blocker !== null}
          title={blocker ?? 'Estimate, confirm and generate'}
          onClick={() => void gen.openConfirm(activeJob)}
        >
          Generate
        </button>
      )}
      {blocker && (
        <p className="muted small" data-testid="gen-panel-submit-reason">
          {blocker}
        </p>
      )}
      {s.submitError && (
        <p className="error small" data-testid="gen-panel-submit-error">
          {errorText(s.submitError.code, s.submitError.message)}
        </p>
      )}
    </div>
  );
}

export function NewImageTab() {
  useGenSession();
  const s = useGen((x) => x);
  const { project } = useProjectState();
  if (!s.projectId) return null;
  const est = s.estimate.result;
  return (
    <div className="gen-tab" data-testid="gen-panel" data-kind={s.kind}>
      {s.readOnly && (
        <p className="rgn-warning small" data-testid="gen-panel-readonly">
          Read-only project: you can look at the settings, but not generate or pick.
        </p>
      )}
      {s.info && (
        <p className="muted small" data-testid="gen-panel-info" role="status">
          {s.info}
        </p>
      )}
      <ModelSection s={s} />
      <SizeSection s={s} />
      <SketchSection s={s} />
      <ReferencesBlock
        host="gen"
        projectId={s.projectId}
        draftReferences={project?.draft?.generate?.references}
        modelId={s.caps ? s.modelId : null}
        hasSlots={(s.caps?.slots.length ?? 0) > 0}
        readOnly={!!s.readOnly}
        structure={takesSketch(s) ? s.sketch : null}
        onChange={gen.referencesChanged}
      />
      <PromptSection s={s} />
      <CostSection s={s} />
      <SubmitSection s={s} />
      <SpendConfirm
        open={s.confirmOpen}
        estimate={est}
        spend={s.spend}
        submitting={s.submitting}
        error={s.confirmError}
        summary={
          est && (
            <p className="gen-confirm-summary tabular" data-testid="gen-confirm-summary">
              New image · {confirmSize(est.output)}
            </p>
          )
        }
        onClose={() => gen.closeConfirm()}
        onConfirm={(plan) => void gen.confirm(plan)}
      />
    </div>
  );
}
