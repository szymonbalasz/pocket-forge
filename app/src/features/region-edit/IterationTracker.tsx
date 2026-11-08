// The iteration tracker (bottom strip, FR-RGN-40): one chip per iteration, newest first, plus the
// active job before it has candidates. The newest unresolved job's chip is the job strip
// (FR-RGN-18..21): state, progress, elapsed, cost, Cancel / Retry / Dismiss. Selecting a chip
// shows its candidates in EDT's picker (R19), which renders below in its own slot.
import { useEffect, useState, type KeyboardEvent } from 'react';

import { IconCheck } from '../../shell/icons';
import { costLabel, elapsedLabel, iterationNumber, jobErrorText, STATE_LABEL } from './format';
import { rgn, useRgn, type LiveJob } from './store';
import { canCancel, canDismiss, canRetry, isActive, orderRows, stripJobId } from './tracker';
import type { RgnIteration } from '../../api';

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
  const state = live?.state ?? row.state;
  const running = isActive(state);
  const now = useNow(running);
  const progress = live?.progress ?? null;
  const error = live?.error ?? null;
  const longRunning = live?.longRunning || (running && !!live && now - live.startedAt >= 180_000);
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
      {state === 'lost' && (
        <div className="warning small" data-testid="rgn-job-lost">
          The job may have been charged, but its result can’t be recovered.
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
        {canRetry(state) && (
          <button
            type="button"
            data-testid="rgn-job-retry"
            onClick={stop(() => void rgn.retry(row.job_id))}
          >
            Retry
          </button>
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
      </div>
    </div>
  );
}

export function IterationTracker() {
  const s = useRgn((x) => x);
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
          const state = live?.state ?? r.state;
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
                </div>
              )}
            </div>
          );
        })}
      </div>
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
