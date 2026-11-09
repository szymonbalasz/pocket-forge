// The iteration tracker (bottom strip, FR-RGN-40): one chip per iteration, newest first, plus the
// active job before it has candidates. The newest unresolved job's chip is the job strip
// (FR-RGN-18..21): state, progress, elapsed, cost, Cancel / Retry / Dismiss. Selecting a chip
// shows its candidates in EDT's picker (R19), which renders below in its own slot.
import { useEffect, useState, type KeyboardEvent } from 'react';
import { Tooltip } from 'radix-ui';

import { IconCheck } from '../../shell/icons';
import { projectStore, useProjectState } from '../../store/project';
import {
  chargeHint,
  costLabel,
  elapsedLabel,
  iterationNumber,
  jobErrorText,
  lostText,
  STATE_LABEL,
} from './format';
import { rgn, useRgn, type LiveJob } from './store';
import {
  canCancel,
  canDismiss,
  canFetchAgain,
  canRetry,
  isActive,
  orderRows,
  outputLabel,
  refsTooltip,
  offersRetry,
  shownState,
  stripJobId,
} from './tracker';
import type { CandidateRecord, RgnIteration } from '../../api';

/** Ticks once a second while `on` (the elapsed time of a running job). */
export function useNow(on: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [on]);
  return now;
}

function JobStrip({ row, live }: { row: RgnIteration; live: LiveJob | undefined }) {
  const state = shownState(row, live);
  const running = isActive(state);
  const now = useNow(running);
  const progress = live?.progress ?? null;
  const error = live?.error ?? null;
  const longRunning = live?.longRunning || (running && !!live && now - live.startedAt >= 180_000);
  // R109: a paid job whose fetch failed may be dismissed, after a confirm.
  const [askDismiss, setAskDismiss] = useState(false);
  const fetchFailed = canFetchAgain(state, error);
  const stop = (fn: () => void) => (e: React.MouseEvent) => {
    e.stopPropagation();
    fn();
  };
  return (
    <div className="rgn-strip" data-testid="rgn-job-strip" data-state={state}>
      <div className="rgn-strip-line">
        <span className={`rgn-state rgn-state-${state}`} data-testid="rgn-job-state">
          {STATE_LABEL[state]}
        </span>
        {running && progress !== null && (
          <span className="tabular" data-testid="rgn-job-progress">
            {Math.round(progress * 100)} %
          </span>
        )}
        {running && live && (
          <span className="muted tabular" data-testid="rgn-job-elapsed">
            {elapsedLabel(now - live.startedAt)}
          </span>
        )}
        <span className="muted tabular" data-testid="rgn-job-cost">
          {costLabel(live?.cost ?? row.cost)}
        </span>
      </div>
      {running && (
        <div className="rgn-progress" aria-hidden="true">
          <div style={{ width: `${Math.round((progress ?? 0) * 100)}%` }} />
        </div>
      )}
      {longRunning && (
        <div className="muted small" data-testid="rgn-job-long">
          Still running. Large jobs can take several minutes.
        </div>
      )}
      {error && state === 'failed' && (
        <div className="error small" data-testid="rgn-job-error" data-code={error.code}>
          {jobErrorText(error)}
        </div>
      )}
      {error && canFetchAgain(state, error) && (
        <div className="error small" data-testid="rgn-job-fetch-error" data-code={error.code}>
          {jobErrorText(error)}
        </div>
      )}
      {state === 'lost' && (
        <div
          className="warning small"
          data-testid="rgn-job-lost"
          data-handle={row.handle_kept ? 'kept' : 'none'}
        >
          {lostText(!!row.handle_kept)}
        </div>
      )}
      <div className="rgn-strip-buttons">
        {canCancel(state) && (
          <button
            type="button"
            data-testid="rgn-job-cancel"
            onClick={stop(() => void rgn.cancelJob(row.job_id))}
          >
            Cancel
          </button>
        )}
        {canFetchAgain(state, error) && (
          <button
            type="button"
            data-testid="rgn-job-fetch-again"
            onClick={stop(() => void rgn.fetchAgain(row.job_id))}
          >
            Fetch again
          </button>
        )}
        {canRetry(state) && (
          <button
            type="button"
            data-testid="rgn-job-retry"
            onClick={stop(() => void rgn.retry(row.job_id))}
          >
            Retry
          </button>
        )}
        {/* R108: Retry stays on a lost job (a pre-fill plus a new confirm), with the
            possible-charge hint next to it. */}
        {state === 'lost' && (
          <span className="warning small" data-testid="rgn-job-charge-hint">
            {chargeHint(error ?? row.error)}
          </span>
        )}
        {canDismiss(state) && (
          <button
            type="button"
            data-testid="rgn-job-dismiss"
            onClick={stop(() => void rgn.dismiss(row.job_id))}
          >
            Dismiss
          </button>
        )}
        {fetchFailed && !askDismiss && (
          <button
            type="button"
            data-testid="rgn-job-dismiss"
            onClick={stop(() => setAskDismiss(true))}
          >
            Dismiss
          </button>
        )}
      </div>
      {fetchFailed && askDismiss && (
        <div className="warning small rgn-strip-buttons" data-testid="rgn-job-dismiss-ask">
          You may have paid for this; its results can’t be downloaded.
          <button
            type="button"
            className="prm-danger"
            data-testid="rgn-job-dismiss-confirm"
            onClick={stop(() => {
              setAskDismiss(false);
              void rgn.dismiss(row.job_id);
            })}
          >
            Dismiss
          </button>
          <button
            type="button"
            data-testid="rgn-job-dismiss-cancel"
            onClick={stop(() => setAskDismiss(false))}
          >
            Cancel
          </button>
        </div>
      )}
    </div>
  );
}

/** FR-RGN-45: `N refs` on a chip whose job sent references, with the files in a tooltip. */
function RefsChip({ row }: { row: RgnIteration }) {
  const text = refsTooltip(row);
  return (
    <Tooltip.Provider>
      <Tooltip.Root>
        <Tooltip.Trigger asChild>
          <span
            className="rgn-tracker-refs muted small tabular"
            data-testid="rgn-tracker-refs"
            data-tooltip={text}
            aria-label={`${row.references} references: ${text.replace(/\n/g, ', ')}`}
          >
            {row.references} {row.references === 1 ? 'ref' : 'refs'}
          </span>
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content className="pf-tooltip rgn-refs-tooltip" side="top" sideOffset={6}>
            {text}
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>
    </Tooltip.Provider>
  );
}

/** FR-GEN-25: a generate iteration's `new image` badge and its sent size. */
/** FR-GEN-25: the sent size; on an aspect model the real W × H once a candidate is in
 *  (FR-GEN-09). */
function NewImageBadge({ row, result }: { row: RgnIteration; result: CandidateRecord | null }) {
  const size =
    row.output?.mode === 'aspect' && result?.width && result.height
      ? `${result.width} × ${result.height}`
      : outputLabel(row.output);
  return (
    <span className="pf-chip rgn-tracker-new" data-testid="gen-tracker-badge">
      new image
      {size && <span className="tabular"> · {size}</span>}
    </span>
  );
}

export function IterationTracker() {
  const s = useRgn((x) => x);
  const trash = useProjectState().trashNotice;
  if (!s.projectId || s.iterations.length === 0) return null;
  const rows = orderRows(s.iterations);
  const strip = stripJobId(rows);
  const select = (id: string) => rgn.selectIteration(id);
  return (
    <div className="rgn-tracker" data-testid="rgn-tracker">
      <div
        className="rgn-tracker-list"
        role="listbox"
        aria-label="Iterations"
        aria-orientation="horizontal"
      >
        {rows.map((r) => {
          const live = s.live[r.job_id];
          const state = shownState(r, live);
          return (
            <div
              key={r.job_id}
              role="option"
              tabIndex={0}
              aria-selected={r.job_id === s.selectedJobId}
              className={`rgn-tracker-item${r.job_id === strip ? ' rgn-tracker-active' : ''}`}
              data-testid="rgn-tracker-item"
              data-record-id={r.job_id}
              data-state={state}
              data-n={r.n ?? ''}
              data-accepted={r.accepted_in_stack ? 'true' : 'false'}
              onClick={() => select(r.job_id)}
              onKeyDown={(e: KeyboardEvent) => {
                if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
                  e.preventDefault();
                  select(r.job_id);
                }
              }}
            >
              <div className="rgn-tracker-head">
                <span className="tabular rgn-tracker-n">
                  {r.n === null ? 'New' : iterationNumber(r.n)}
                </span>
                <span className="rgn-tracker-slug" title={r.dir ?? undefined}>
                  {r.slug ?? 'job'}
                </span>
                {r.operation === 'generate' && (
                  <NewImageBadge
                    row={r}
                    result={
                      s.candidates.find(
                        (c) => c.job_id === r.job_id && c.status !== 'invalid' && !!c.width,
                      ) ?? null
                    }
                  />
                )}
                {(r.references ?? 0) > 0 && <RefsChip row={r} />}
                {r.accepted_in_stack && (
                  <span
                    className="rgn-tracker-check"
                    data-testid="rgn-tracker-accepted"
                    title="Accepted patch in the current stack"
                  >
                    <IconCheck size={12} />
                  </span>
                )}
              </div>
              {r.job_id === strip ? (
                <JobStrip row={r} live={live} />
              ) : (
                <div className="rgn-tracker-meta muted small tabular">
                  <span>{STATE_LABEL[state]}</span>
                  {r.returned > 0 && (
                    <span data-testid="rgn-tracker-kept">
                      {r.kept}/{r.returned} kept
                    </span>
                  )}
                  <span>{costLabel(r.cost)}</span>
                  {offersRetry(rows, r) && (
                    <button
                      type="button"
                      className="small"
                      data-testid="rgn-job-retry"
                      data-record-id={r.job_id}
                      onClick={(e) => {
                        e.stopPropagation();
                        void rgn.retry(r.job_id);
                      }}
                    >
                      Retry
                    </button>
                  )}
                  {state === 'lost' && offersRetry(rows, r) && (
                    <span
                      className="warning"
                      data-testid="rgn-job-charge-hint"
                      data-record-id={r.job_id}
                    >
                      {chargeHint(live?.error ?? r.error)}
                    </span>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
      {trash?.at === 'tracker' && !s.trackerNotice && (
        <div
          className="small rgn-tracker-notice muted"
          data-testid="rgn-tracker-notice"
          role="status"
        >
          {trash.text}
          <button
            type="button"
            className="link small"
            onClick={() => projectStore.setTrashNotice(null)}
          >
            ×
          </button>
        </div>
      )}
      {s.trackerNotice && (
        <div
          className={`small rgn-tracker-notice ${s.trackerNotice.kind === 'error' ? 'error' : 'muted'}`}
          data-testid="rgn-tracker-notice"
          role="status"
        >
          {s.trackerNotice.text}
          <button type="button" className="link small" onClick={() => rgn.clearTrackerNotice()}>
            ×
          </button>
        </div>
      )}
    </div>
  );
}
