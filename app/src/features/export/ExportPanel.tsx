// The Export section of the right panel (EXP §7): grid input, target px per square, format,
// the plan line, Start / Replace, progress with Cancel, errors with Retry, and the done toast.
// `Ctrl+E` expands the section and focuses it.
import { useEffect, useRef, useState } from 'react';
import { Collapsible, RadioGroup, Toast, ToggleGroup, Tooltip } from 'radix-ui';

import type { ExportFormat } from '../../api';
import { useProjectState } from '../../store/project';
import { exportStore, useExportState } from './store';
import {
  fileNameOf,
  formatBytes,
  formHasSource,
  PHASE_TEXT,
  planLine,
  replaceNote,
  startState,
  type FieldName,
} from './view';
import './export.css';

const FORMATS: { format: ExportFormat; label: string }[] = [
  { format: 'png', label: 'PNG' },
  { format: 'jpeg', label: 'JPEG' },
  { format: 'webp', label: 'WebP' },
];

/** True for Ctrl+E (Meta counts as Ctrl) without Shift or Alt. */
export function isExportChord(e: KeyboardEvent): boolean {
  return (e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'e';
}

function Field({
  id,
  label,
  value,
  disabled,
  error,
  inputMode,
  testId,
}: {
  id: FieldName;
  label: string;
  value: string;
  disabled?: boolean;
  error: string | null;
  inputMode: 'decimal' | 'numeric';
  testId: string;
}) {
  return (
    <label className="exp-field">
      <span className="exp-field-label small muted">{label}</span>
      <input
        type="text"
        inputMode={inputMode}
        className="tabular"
        name={id}
        value={value}
        disabled={disabled}
        aria-invalid={error ? true : undefined}
        data-testid={testId}
        onChange={(e) => void exportStore.edit({ [id]: e.target.value })}
      />
      {error && (
        <span className="small error" data-testid={`${testId}-error`}>
          {error}
        </span>
      )}
    </label>
  );
}

function ExportToast() {
  const { done } = useExportState();
  const [copied, setCopied] = useState<'idle' | 'copied' | 'failed'>('idle');
  useEffect(() => setCopied('idle'), [done]);

  const copy = async () => {
    if (!done) return;
    try {
      await navigator.clipboard.writeText(done.path);
      setCopied('copied');
    } catch {
      setCopied('failed');
    }
  };

  return (
    <Toast.Root
      className="exp-toast"
      data-testid="exp-toast"
      open={!!done}
      duration={10_000}
      onOpenChange={(open) => !open && exportStore.dismissDone()}
    >
      {done && (
        <>
          <Toast.Title className="exp-toast-title">Exported {fileNameOf(done.path)}</Toast.Title>
          <Toast.Description className="small muted tabular" data-testid="exp-toast-detail">
            {done.width}×{done.height} · {formatBytes(done.bytes)}
            <span className="exp-toast-path" data-testid="exp-toast-path">
              {done.path}
            </span>
          </Toast.Description>
          {done.fallback === 'lossless' && (
            <p className="small warning" data-testid="exp-toast-fallback">
              Saved as lossless WebP: the lossy encoder couldn't handle this image.
            </p>
          )}
          {!done.record_saved && (
            <p className="small warning" data-testid="exp-toast-unsaved">
              The file is saved, but the project's export record isn't yet. Retrying.
            </p>
          )}
          <div className="row">
            <button type="button" data-testid="exp-toast-copy" onClick={() => void copy()}>
              {copied === 'copied' ? 'Copied' : copied === 'failed' ? "Couldn't copy" : 'Copy path'}
            </button>
            <Toast.Close asChild>
              <button type="button" className="link" data-testid="exp-toast-close">
                Close
              </button>
            </Toast.Close>
          </div>
        </>
      )}
    </Toast.Root>
  );
}

export function ExportPanel() {
  const { project } = useProjectState();
  const s = useExportState();
  const body = useRef<HTMLDivElement>(null);
  const projectId = project?.project_id ?? null;

  useEffect(() => {
    void exportStore.listen();
  }, []);
  useEffect(() => {
    void exportStore.bind(projectId);
  }, [projectId]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isExportChord(e)) return;
      e.preventDefault();
      exportStore.focus();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  useEffect(() => {
    if (!s.focusTick) return;
    const id = requestAnimationFrame(() => {
      const el = body.current?.querySelector<HTMLElement>(
        '[data-testid="exp-panel-grid-mode"] [data-state="on"], input:not(:disabled)',
      );
      el?.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [s.focusTick]);

  const { form, plan, run } = s;
  const hasSource = formHasSource(form);
  const err = (f: FieldName) => (s.fieldError?.field === f ? s.fieldError.message : null);
  const start = startState({
    hasProject: !!project,
    readOnly: project?.read_only ?? null,
    original: s.original,
    plan,
    format: form.format,
    running: !!run,
    blocked: !!s.fieldError || !!s.planError,
  });
  const note = plan ? replaceNote(plan, Date.now()) : null;
  const panelState = !project
    ? 'empty'
    : project.read_only
      ? 'read-only'
      : run
        ? 'running'
        : s.error
          ? 'error'
          : s.original.kind === 'loading'
            ? 'loading'
            : 'ready';

  return (
    <Toast.Provider swipeDirection="right">
      <Collapsible.Root
        open={s.expanded}
        onOpenChange={(open) => exportStore.setExpanded(open)}
        className="exp-panel"
        data-testid="exp-panel"
        data-state-panel={panelState}
      >
        <Collapsible.Trigger asChild>
          <button type="button" className="exp-panel-header" data-testid="exp-panel-toggle">
            <span className="exp-chevron" aria-hidden>
              {s.expanded ? '▾' : '▸'}
            </span>
            <span>Export</span>
            {run && (
              <span className="small muted tabular exp-header-progress">
                {Math.round(run.fraction * 100)} %
              </span>
            )}
          </button>
        </Collapsible.Trigger>
        <Collapsible.Content className="exp-panel-body" ref={body}>
          {!project && (
            <p className="small muted" data-testid="exp-panel-empty">
              Open a project to export.
            </p>
          )}
          <fieldset className="exp-fieldset" disabled={!project}>
            <div className="exp-row">
              <span className="small muted">Grid</span>
              <ToggleGroup.Root
                type="single"
                className="exp-toggle"
                value={form.mode}
                aria-label="Grid input"
                data-testid="exp-panel-grid-mode"
                onValueChange={(v) => v && void exportStore.edit({ mode: v as 'px' | 'cells' })}
              >
                <ToggleGroup.Item value="px" data-testid="exp-panel-grid-mode-px">
                  Px per square
                </ToggleGroup.Item>
                <ToggleGroup.Item value="cells" data-testid="exp-panel-grid-mode-cells">
                  Cells
                </ToggleGroup.Item>
              </ToggleGroup.Root>
            </div>
            {form.mode === 'px' ? (
              <div className="exp-row">
                <Field
                  id="source"
                  label="Source px per square"
                  value={form.source}
                  inputMode="decimal"
                  error={err('source')}
                  testId="exp-panel-grid-source"
                />
              </div>
            ) : (
              <div className="exp-row">
                <Field
                  id="cols"
                  label="Columns"
                  value={form.cols}
                  inputMode="numeric"
                  error={err('cols')}
                  testId="exp-panel-grid-cols"
                />
                <Field
                  id="rows"
                  label="Rows (optional)"
                  value={form.rows}
                  inputMode="numeric"
                  error={err('rows')}
                  testId="exp-panel-grid-rows"
                />
              </div>
            )}
            <div className="exp-row">
              <Field
                id="target"
                label="Target px per square"
                value={form.target}
                inputMode="numeric"
                disabled={!hasSource}
                error={
                  err('target') ?? (s.fieldError?.field === null ? s.fieldError.message : null)
                }
                testId="exp-panel-target-ppsq"
              />
            </div>
            <RadioGroup.Root
              className="exp-formats"
              value={form.format}
              aria-label="Format"
              orientation="horizontal"
              onValueChange={(v) => void exportStore.edit({ format: v as ExportFormat })}
            >
              {FORMATS.map(({ format, label }) => {
                const f = plan?.formats.find((x) => x.format === format);
                const unavailable = f && !f.available ? (f.reason ?? 'Unavailable') : null;
                const item = (
                  <label className="exp-format" data-disabled={unavailable ? '' : undefined}>
                    <RadioGroup.Item
                      value={format}
                      className="exp-radio"
                      disabled={!!unavailable}
                      data-testid={`exp-panel-format-${format}`}
                      aria-description={unavailable ?? undefined}
                    >
                      <RadioGroup.Indicator className="exp-radio-dot" />
                    </RadioGroup.Item>
                    <span>{label}</span>
                  </label>
                );
                return unavailable ? (
                  <Tooltip.Root key={format}>
                    <Tooltip.Trigger asChild>
                      <span data-testid={`exp-panel-format-${format}-reason`} title={unavailable}>
                        {item}
                      </span>
                    </Tooltip.Trigger>
                    <Tooltip.Portal>
                      <Tooltip.Content className="pf-tooltip" side="top">
                        {unavailable}
                      </Tooltip.Content>
                    </Tooltip.Portal>
                  </Tooltip.Root>
                ) : (
                  <span key={format}>{item}</span>
                );
              })}
            </RadioGroup.Root>
          </fieldset>

          {project && (
            <div className="exp-plan">
              <p className="small tabular" data-testid="exp-panel-plan">
                {plan
                  ? planLine(project.width, project.height, plan)
                  : (s.planError ?? (s.fieldError ? 'Fix the settings above' : '…'))}
              </p>
              {plan && plan.warnings.length > 0 && (
                <ul className="exp-warnings small warning" data-testid="exp-panel-warnings">
                  {plan.warnings.map((w) => (
                    <li key={w.code} data-record-id={w.code}>
                      {w.message}
                    </li>
                  ))}
                </ul>
              )}
              {note && !run && (
                <p className="small muted" data-testid="exp-panel-replace-note">
                  {note}
                </p>
              )}
            </div>
          )}

          {run ? (
            <div className="exp-progress-row">
              <div
                className="exp-progress"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(run.fraction * 100)}
                data-testid="exp-panel-progress"
                data-phase={run.phase}
              >
                <div className="exp-progress-fill" style={{ width: `${run.fraction * 100}%` }} />
              </div>
              <span className="small muted tabular" data-testid="exp-panel-phase">
                {run.canceling
                  ? 'Canceling…'
                  : `${PHASE_TEXT[run.phase] ?? run.phase} ${Math.round(run.fraction * 100)} %`}
              </span>
              <button
                type="button"
                data-testid="exp-panel-cancel"
                disabled={run.canceling || !run.exportId}
                onClick={() => void exportStore.cancel()}
              >
                Cancel
              </button>
            </div>
          ) : (
            project && (
              <div className="exp-actions">
                <button
                  type="button"
                  className="primary"
                  data-testid="exp-panel-start"
                  disabled={!start.enabled}
                  onClick={() => void exportStore.start()}
                >
                  {start.label}
                </button>
                {start.reason && (
                  <span className="small muted" data-testid="exp-panel-start-reason">
                    {start.reason}
                  </span>
                )}
              </div>
            )
          )}

          {s.error && !run && (
            <div
              className="exp-error banner banner-error small"
              data-testid="exp-panel-error"
              data-code={s.error.code}
            >
              <span>{s.error.message}</span>
              {s.error.code !== 'read_only' && (
                <button
                  type="button"
                  data-testid="exp-panel-retry"
                  onClick={() => void exportStore.retry()}
                >
                  Retry
                </button>
              )}
            </div>
          )}
        </Collapsible.Content>
      </Collapsible.Root>
      <ExportToast />
      <Toast.Viewport className="exp-toast-viewport" />
    </Toast.Provider>
  );
}

/** Status-bar segment: export progress while one runs (ui-guidelines: long operations). */
export function ExportStatus() {
  const { run } = useExportState();
  if (!run) return null;
  return (
    <span className="small tabular" data-testid="exp-status-progress">
      Exporting {Math.round(run.fraction * 100)} %
    </span>
  );
}
