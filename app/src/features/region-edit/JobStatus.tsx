// RGN's status-bar segment (ui-guidelines): the newest job's state and progress, hidden when
// there is no job to report.
import { STATE_LABEL } from './format';
import { useRgn } from './store';
import { orderRows, shownState, stripJobId } from './tracker';

export function JobStatus() {
  const s = useRgn((x) => x);
  if (!s.projectId) return null;
  const id = stripJobId(s.iterations);
  if (!id) return null;
  const row = orderRows(s.iterations)[0]!;
  const live = s.live[id];
  const state = shownState(row, live);
  const progress = live?.progress ?? null;
  return (
    <span
      className={`rgn-status rgn-state-${state}`}
      data-testid="rgn-status-job"
      data-state={state}
    >
      Job: {STATE_LABEL[state]}
      {(state === 'running' || state === 'queued') && progress !== null
        ? ` ${Math.round(progress * 100)} %`
        : ''}
    </span>
  );
}
