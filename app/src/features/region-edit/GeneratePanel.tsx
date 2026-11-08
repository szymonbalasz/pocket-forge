// The Generate section of the right panel (RGN §7), top to bottom: Region (size, margin, fit
// label) · Model (picker, capability controls per Appendix A, warnings) · Prompt · Seed · Cost
// (estimate, session spend / cap) · Submit · blend controls while a candidate is selected.
import { useEffect, useState } from 'react';
import { Collapsible, Select, Slider, Switch } from 'radix-ui';

import { useSlotIdle } from '../../shared/slot';
import { useProjectState } from '../../store/project';
import { useEditorMaskPainted, useEditorRegion, useEditorRegionRestored } from '../editor';
import { openProviderSettings } from '../provider';
import { ReferencesBlock } from '../references';
import { ConfirmDialog } from './ConfirmDialog';
import { ControlField } from './ControlField';
import { controlKind, isValueControl, stateOf } from './controls';
import { amountLabel, costLabel, errorText, fitLabel } from './format';
import {
  currentDraft,
  costUnit,
  finalPrompt,
  isOffline,
  MASK_HINT,
  missing,
  rgn,
  selectedLayer,
  submitBlocker,
  useRgn,
  type RgnState,
} from './store';
import './rgn.css';

/** Binds the store to the open project and to EDT's region (R18). EDT's region is followed only
 *  once EDT has restored this project's saved region: before that it is null or the previous
 *  project's, and must not reach this project's draft (walk D2). */
function useRgnSession(): void {
  const { project } = useProjectState();
  const region = useEditorRegion();
  const restoredFor = useEditorRegionRestored();
  const painted = useEditorMaskPainted();
  const pid = project?.project_id ?? null;
  useEffect(() => {
    if (project) void rgn.attach(project);
    else rgn.detach();
    // Re-attach only per project / read-only change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pid, project?.read_only]);
  useEffect(() => {
    if (pid && restoredFor === pid && rgn.get().projectId === pid)
      rgn.editorRegion(region?.inner ?? null);
  }, [pid, restoredFor, region?.inner]);
  // Owner 2026-11-08: Generate waits for paint in the region.
  useEffect(() => rgn.setMaskPainted(painted), [pid, painted]);
}

function RegionSection({ s }: { s: RgnState }) {
  if (!s.inner)
    return (
      <p className="muted" data-testid="rgn-panel-empty">
        Drag a box to start a region edit (R).
      </p>
    );
  return (
    <div className="rgn-section" data-testid="rgn-panel-region">
      <div className="rgn-row">
        <span className="muted">Region</span>
        <span className="tabular" data-testid="rgn-panel-size">
          {s.inner.w} × {s.inner.h} px
        </span>
      </div>
      <label className="rgn-control-label" htmlFor="rgn-margin">
        Context margin
      </label>
      <MarginControl margin={s.margin} />
      {s.fit && (
        <div className="rgn-row small">
          <span className="muted tabular">
            sent {s.fit.sent_w} × {s.fit.sent_h}
          </span>
          <span data-testid="rgn-panel-fit">{fitLabel(s.fit)}</span>
        </div>
      )}
      {s.fitError && (
        <p className="error small" data-testid="rgn-panel-fit-error">
          {errorText(s.fitError.code, s.fitError.message)}
        </p>
      )}
    </div>
  );
}

/** FR-RGN-02: 0–512 px; the slider refits on release, the field on Enter or blur. */
function MarginControl({ margin }: { margin: number }) {
  const [drag, setDrag] = useState<number | null>(null);
  return (
    <div className="rgn-slider-row">
      <Slider.Root
        className="rgn-slider"
        min={0}
        max={512}
        step={8}
        value={[drag ?? margin]}
        onValueChange={([v]) => setDrag(v ?? null)}
        onValueCommit={([v]) => {
          setDrag(null);
          if (v !== undefined) rgn.setMargin(v);
        }}
      >
        <Slider.Track className="rgn-slider-track">
          <Slider.Range className="rgn-slider-range" />
        </Slider.Track>
        <Slider.Thumb className="rgn-slider-thumb" aria-label="Context margin" />
      </Slider.Root>
      <MarginInput margin={drag ?? margin} />
    </div>
  );
}

function MarginInput({ margin }: { margin: number }) {
  const [text, setText] = useState(String(margin));
  useEffect(() => setText(String(margin)), [margin]);
  const commit = () => {
    const n = Number(text);
    if (Number.isFinite(n)) rgn.setMargin(n);
    else setText(String(margin));
  };
  return (
    <input
      id="rgn-margin"
      type="number"
      min={0}
      max={512}
      className="rgn-number tabular"
      data-testid="rgn-panel-margin"
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === 'Enter' && commit()}
    />
  );
}

function ModelSection({ s }: { s: RgnState }) {
  const warnings = [...(s.caps?.warnings ?? []), ...s.notices];
  return (
    <div className="rgn-section">
      <label className="rgn-control-label" htmlFor="rgn-model">
        Model
        {s.modelsStale && (
          <span className="pf-chip" data-testid="rgn-panel-stale" title="Offline: cached list">
            cached
          </span>
        )}
      </label>
      <Select.Root value={s.modelId ?? ''} onValueChange={(id) => void rgn.pickModel(id)}>
        <Select.Trigger
          className="rgn-select"
          id="rgn-model"
          aria-label="Model"
          data-testid="rgn-panel-model"
          disabled={s.models.length === 0}
        >
          <Select.Value placeholder={s.modelsLoading ? 'Loading models…' : 'No models'} />
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
                  data-testid="rgn-panel-model-option"
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
        <p className="muted small" data-testid="rgn-panel-loading">
          Loading capabilities…
        </p>
      )}
      {s.capsError && (
        <p className="error small">{errorText(s.capsError.code, s.capsError.message)}</p>
      )}
      {s.caps && (
        <div className="rgn-controls" data-testid="rgn-panel-controls">
          {s.caps.controls
            .filter((c) => isValueControl(c) && (stateOf(s.evaluation, c.name)?.visible ?? true))
            .map((c) => (
              <ControlField
                key={c.name}
                c={c}
                value={s.values[c.name]}
                state={stateOf(s.evaluation, c.name)}
                onChange={(v) => rgn.setValue(c.name, v)}
              />
            ))}
          {!s.caps.controls.some((c) => c.role === 'candidate_count') && (
            // Owner 2026-11-08: no count control → one result per job, never N jobs.
            <p className="muted small" data-testid="rgn-panel-count-single">
              1 result per job (this model)
            </p>
          )}
        </div>
      )}
      {warnings.length > 0 && (
        <div className="rgn-warning" data-testid="rgn-panel-warning" role="status">
          <ul>
            {warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
          {s.notices.length > 0 && (
            <button type="button" className="link small" onClick={() => rgn.dismissNotices()}>
              Dismiss notices
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function PromptSection({ s }: { s: RgnState }) {
  const required = !!s.caps?.prompt_required;
  const seedCtl = s.caps?.controls.find((c) => controlKind(c) === 'seed');
  const draft = currentDraft(s);
  return (
    <div className="rgn-section">
      <label className="rgn-control-label" htmlFor="rgn-prompt">
        Prompt
        {required && (
          <span className="rgn-required" title="Required">
            *
          </span>
        )}
      </label>
      <textarea
        id="rgn-prompt"
        className="rgn-prompt"
        data-testid="rgn-panel-prompt"
        rows={4}
        value={s.prompt}
        placeholder="Describe the change, e.g. turn this room into a kitchen"
        onChange={(e) => rgn.setPrompt(e.target.value)}
      />
      <label className="rgn-check" title="Appends the blend instruction (Settings) to the prompt">
        <Switch.Root
          className="rgn-switch"
          aria-label="Blend with surroundings"
          data-testid="rgn-panel-blend-instruction"
          checked={s.blendInstruction}
          onCheckedChange={(v) => rgn.setBlendInstruction(v)}
        >
          <Switch.Thumb className="rgn-switch-thumb" />
        </Switch.Root>
        Blend with surroundings
      </label>
      {draft && (
        <details className="rgn-final" data-testid="rgn-panel-final">
          <summary className="muted small">Final prompt (as sent)</summary>
          <pre className="small" data-testid="rgn-panel-final-prompt">
            {finalPrompt(s) || '—'}
          </pre>
        </details>
      )}
      {s.caps?.seed && seedCtl && (
        <div className="rgn-row" data-testid="rgn-panel-seed">
          <span className="muted">Seed</span>
          <label className="rgn-check">
            <Switch.Root
              className="rgn-switch"
              aria-label="Pin the seed"
              data-testid="rgn-panel-seed-pin"
              checked={s.seedMode === 'pinned'}
              onCheckedChange={(v) => rgn.setSeedMode(v ? 'pinned' : 'random')}
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
              data-testid="rgn-panel-seed-value"
              value={s.seed ?? 0}
              onChange={(e) =>
                rgn.setSeed(Number.isFinite(Number(e.target.value)) ? Number(e.target.value) : null)
              }
            />
          )}
        </div>
      )}
    </div>
  );
}

function CostSection({ s }: { s: RgnState }) {
  const e = s.estimate;
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
  const unit = costUnit(s);
  return (
    <div className="rgn-section">
      <div
        className="rgn-estimate"
        data-testid="rgn-panel-estimate"
        data-status={s.readOnly || isOffline(s) ? 'idle' : e.status}
        data-cost={e.result ? String(e.result.cost.amount) : ''}
      >
        {line}
      </div>
      {s.spend && (
        <div className="muted small tabular" data-testid="rgn-panel-spend">
          Session {amountLabel(s.spend.spend, unit)}
          {s.spend.cap !== null ? ` of ${amountLabel(s.spend.cap, unit)} cap` : ' · no cap set yet'}
        </div>
      )}
    </div>
  );
}

function SubmitSection({ s }: { s: RgnState }) {
  const blocker = submitBlocker(s);
  const notConfigured = s.provider && !s.provider.configured;
  const authError = s.provider?.connectivity === 'auth_error' || s.estimate.error?.code === 'auth';
  const m = missing(s);
  return (
    <div className="rgn-section">
      {notConfigured || authError ? (
        <button
          type="button"
          className="primary rgn-submit"
          data-testid="rgn-panel-setup"
          onClick={openProviderSettings}
        >
          Set up provider
        </button>
      ) : (
        <button
          type="button"
          className="primary rgn-submit"
          data-testid="rgn-panel-submit"
          disabled={blocker !== null}
          title={blocker ?? 'Estimate, confirm and submit'}
          onClick={() => void rgn.openConfirm()}
        >
          Generate
        </button>
      )}
      {blocker === MASK_HINT && s.inner ? (
        // Owner 2026-11-08: what to do when the region has no painted mask.
        <p className="muted small" data-testid="rgn-panel-mask-hint">
          {MASK_HINT}
        </p>
      ) : (
        blocker &&
        s.inner && (
          <p
            className="muted small"
            data-testid="rgn-panel-submit-reason"
            data-missing={m.join(',')}
          >
            {blocker}
          </p>
        )
      )}
      {s.submitError && (
        <p className="error small" data-testid="rgn-panel-submit-error">
          {errorText(s.submitError.code, s.submitError.message)}
        </p>
      )}
    </div>
  );
}

function BlendSection({ s }: { s: RgnState }) {
  if (!s.selectedCandidateId) return null;
  const layer = selectedLayer(s);
  return (
    <div className="rgn-section" data-testid="rgn-panel-blend">
      <label className="rgn-control-label" htmlFor="rgn-feather">
        Feather
      </label>
      <div className="rgn-slider-row">
        <Slider.Root
          className="rgn-slider"
          min={0}
          max={256}
          step={1}
          value={[s.blend.feather_px]}
          onValueChange={([v]) => v !== undefined && rgn.setBlend({ feather_px: v })}
        >
          <Slider.Track className="rgn-slider-track">
            <Slider.Range className="rgn-slider-range" />
          </Slider.Track>
          <Slider.Thumb className="rgn-slider-thumb" aria-label="Feather" />
        </Slider.Root>
        <input
          id="rgn-feather"
          type="number"
          min={0}
          max={256}
          className="rgn-number tabular"
          data-testid="rgn-blend-feather"
          value={s.blend.feather_px}
          onChange={(e) => {
            const n = Math.round(Number(e.target.value));
            if (Number.isFinite(n)) rgn.setBlend({ feather_px: Math.max(0, Math.min(256, n)) });
          }}
        />
      </div>
      <label className="rgn-check">
        <Switch.Root
          className="rgn-switch"
          aria-label="Colour match"
          data-testid="rgn-blend-colour"
          checked={s.blend.colour_match}
          onCheckedChange={(v) => rgn.setBlend({ colour_match: v })}
        >
          <Switch.Thumb className="rgn-switch-thumb" />
        </Switch.Root>
        Colour match
      </label>
      {layer && (
        <button type="button" data-testid="rgn-blend-retune" onClick={() => void rgn.retune()}>
          Re-tune the accepted patch
        </button>
      )}
    </div>
  );
}

export function GeneratePanel() {
  useRgnSession();
  const s = useRgn((x) => x);
  const { project } = useProjectState();
  const [open, setOpen] = useState(true);
  useSlotIdle(!s.projectId);
  if (!s.projectId) return null;
  return (
    <Collapsible.Root open={open} onOpenChange={setOpen} asChild>
      <section className="rgn-panel" data-testid="rgn-panel" aria-label="Generate">
        <Collapsible.Trigger asChild>
          <button type="button" className="rgn-panel-head" data-testid="rgn-panel-toggle">
            <span>{open ? '▾' : '▸'}</span> Generate
          </button>
        </Collapsible.Trigger>
        <Collapsible.Content className="rgn-panel-body">
          {s.readOnly && (
            <p className="rgn-warning small" data-testid="rgn-panel-readonly">
              Read-only project: you can view, pick a region and paint, but not submit or accept.
            </p>
          )}
          <RegionSection s={s} />
          <ModelSection s={s} />
          <ReferencesBlock
            projectId={s.projectId}
            draftReferences={project?.draft?.references}
            modelId={s.caps ? s.modelId : null}
            hasSlots={(s.caps?.slots.length ?? 0) > 0}
            readOnly={!!s.readOnly}
            onChange={rgn.referencesChanged}
          />
          <PromptSection s={s} />
          <CostSection s={s} />
          <SubmitSection s={s} />
          <BlendSection s={s} />
        </Collapsible.Content>
        <ConfirmDialog />
      </section>
    </Collapsible.Root>
  );
}
