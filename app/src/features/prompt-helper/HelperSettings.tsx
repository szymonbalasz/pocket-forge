// Settings → Prompt helpers (prompt-helper §7, FR-PRM-01..03): one list per asset type (Maps,
// Tokens, Items) with label, text, the kinds, "On by default", up/down, Hide (built-ins) or Delete
// (user-made, confirmed inline), and Reset; then + Helper and Reset all. Edits stay local until
// Save, which writes the whole ordered list (FR-PRM-02). Reset acts at once on the stored list.
import { useEffect, useRef, useState } from 'react';

import {
  api,
  ApiError,
  type ProjectKind,
  type PromptHelper,
  type PromptHelperInput,
} from '../../api';
import { KIND_TABS, MAX_HELPERS, TOO_MANY, fieldError, moveWithin } from './logic';
import { setHelpers, useHelpers } from './store';
import './prm.css';

interface Row extends PromptHelperInput {
  /** The helper id, or `new-N` for one not saved yet. */
  key: string;
  builtin: boolean;
}

let newRows = 0;

const toRow = (h: PromptHelper): Row => ({
  key: h.id,
  id: h.id,
  label: h.label,
  text: h.text,
  kinds: [...h.kinds],
  default_on: h.default_on,
  hidden: h.hidden,
  builtin: h.builtin,
});

const toInput = (r: Row): PromptHelperInput => ({
  ...(r.id ? { id: r.id } : {}),
  label: r.label.trim(),
  text: r.text.trim(),
  kinds: r.kinds,
  default_on: r.default_on,
  hidden: r.hidden,
});

const same = (a: readonly Row[], b: readonly Row[]) =>
  JSON.stringify(a.map(toInput)) === JSON.stringify(b.map(toInput));

const rowDiffers = (r: Row, h: PromptHelper) => !same([r], [toRow(h)]);

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function HelperSettings() {
  const { helpers, error } = useHelpers();
  const [rows, setRows] = useState<Row[] | null>(null);
  const [tab, setTab] = useState<ProjectKind>('map');
  /** Rows listed on this tab: those of its kind when it was shown, plus ones added here. A row
   *  whose kinds change stays until the tab changes, so it doesn't vanish while edited. */
  const [shown, setShown] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string; key?: string } | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const base = useRef<PromptHelper[] | null>(null);

  // Adopt the stored list (first load, or a change from elsewhere) unless there are local edits.
  useEffect(() => {
    if (!helpers) return;
    const prev = base.current;
    base.current = helpers;
    setRows((cur) => {
      if (cur && prev && !same(cur, prev.map(toRow))) return cur;
      const next = helpers.map(toRow);
      setShown(next.filter((r) => r.kinds.includes(tab)).map((r) => r.key));
      return next;
    });
    // `tab` is read for the first listing only; a tab change sets `shown` itself.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [helpers]);

  if (!rows || !helpers) {
    return (
      <div className="prm-settings" data-testid="prm-settings">
        {error ? (
          <p className="prv-meta prv-error">Prompt helpers unavailable: {error.message}</p>
        ) : (
          <p className="prv-meta">Loading…</p>
        )}
      </div>
    );
  }

  const dirty = !same(rows, helpers.map(toRow));
  const invalid = rows.some((r) => fieldError(r));
  const visible = shown
    .map((k) => rows.find((r) => r.key === k))
    .filter((r): r is Row => !!r)
    .sort((a, b) => rows.indexOf(a) - rows.indexOf(b));

  const patch = (key: string, p: Partial<Row>) =>
    setRows(rows.map((r) => (r.key === key ? { ...r, ...p } : r)));

  const selectTab = (kind: ProjectKind) => {
    setTab(kind);
    setConfirm(null);
    setShown(rows.filter((r) => r.kinds.includes(kind)).map((r) => r.key));
  };

  const add = () => {
    const key = `new-${++newRows}`;
    setRows([
      ...rows,
      { key, label: '', text: '', kinds: [tab], default_on: false, hidden: false, builtin: false },
    ]);
    setShown([...shown, key]);
    setResult(null);
  };

  const save = async () => {
    setBusy(true);
    setResult(null);
    try {
      const saved = await api.prompt.saveHelpers(rows.map(toInput));
      const next = saved.map(toRow);
      base.current = saved;
      // New rows get their ids: map the listed keys by position.
      const keyMap = new Map(rows.map((r, i) => [r.key, next[i]?.key ?? r.key]));
      setRows(next);
      setShown(shown.map((k) => keyMap.get(k) ?? k));
      setHelpers(saved);
      setResult({ ok: true, text: 'Saved.' });
    } catch (e) {
      if (e instanceof ApiError && e.code === 'too_many') setResult({ ok: false, text: TOO_MANY });
      else {
        const at = e instanceof ApiError ? /^helpers\[(\d+)\]/.exec(e.field ?? '') : null;
        setResult({
          ok: false,
          text: errorText(e),
          key: at ? rows[Number(at[1])]?.key : undefined,
        });
      }
    } finally {
      setBusy(false);
    }
  };

  /** Reset one built-in (FR-PRM-03): stored at once; other local edits are kept. */
  const reset = async (id: string) => {
    setBusy(true);
    setResult(null);
    try {
      const list = await api.prompt.resetHelper(id);
      const fresh = list.find((h) => h.id === id);
      const next = fresh ? rows.map((r) => (r.key === id ? toRow(fresh) : r)) : rows;
      base.current = list;
      setRows(next);
      setHelpers(list);
    } catch (e) {
      setResult({ ok: false, text: errorText(e), key: id });
    } finally {
      setBusy(false);
    }
  };

  const resetAll = async () => {
    setConfirm(null);
    setBusy(true);
    setResult(null);
    try {
      const list = await api.prompt.resetAllHelpers();
      const next = list.map(toRow);
      base.current = list;
      setRows(next);
      setShown(next.filter((r) => r.kinds.includes(tab)).map((r) => r.key));
      setHelpers(list);
      setResult({ ok: true, text: 'Built-in helpers reset.' });
    } catch (e) {
      setResult({ ok: false, text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };

  const full = rows.length >= MAX_HELPERS;

  return (
    <div className="prm-settings" data-testid="prm-settings">
      <p className="prv-meta">
        Ticked helpers add their text to the final prompt, after what you type. Helpers are kept on
        this computer.
      </p>
      <div className="prm-settings-tabs" role="tablist">
        {KIND_TABS.map((t) => (
          <button
            key={t.kind}
            type="button"
            role="tab"
            className="prm-settings-tab"
            data-testid="prm-settings-tab"
            data-record-id={t.kind}
            aria-selected={tab === t.kind}
            onClick={() => selectTab(t.kind)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <ul className="prm-settings-list" data-testid="prm-settings-list">
        {visible.length === 0 && <li className="prv-meta">No helpers for this asset type.</li>}
        {visible.map((r, i) => (
          <HelperRow
            key={r.key}
            row={r}
            first={i === 0}
            last={i === visible.length - 1}
            stored={helpers.find((h) => h.id === r.id)}
            busy={busy}
            confirming={confirm === r.key}
            error={result && !result.ok && result.key === r.key ? result.text : null}
            onPatch={(p) => {
              patch(r.key, p);
              setResult(null);
            }}
            onMove={(dir) => setRows(moveWithin(rows, r.key, tab, dir))}
            onReset={() => void reset(r.key)}
            onDelete={() => setConfirm(r.key)}
            onConfirmDelete={() => {
              setRows(rows.filter((x) => x.key !== r.key));
              setShown(shown.filter((k) => k !== r.key));
              setConfirm(null);
            }}
            onCancelDelete={() => setConfirm(null)}
          />
        ))}
      </ul>
      <div className="prv-actions">
        <button type="button" data-testid="prm-settings-add" disabled={busy || full} onClick={add}>
          + Helper
        </button>
        <span className="pf-spacer" />
        {confirm === 'all' ? (
          <span className="prm-confirm" data-testid="prm-settings-reset-all-ask">
            {dirty ? 'Discard unsaved changes and reset' : 'Reset'} every built-in helper? Your own
            helpers are kept.
            <button
              type="button"
              className="prm-danger"
              data-testid="prm-settings-reset-all-confirm"
              onClick={() => void resetAll()}
            >
              Reset all
            </button>
            <button type="button" onClick={() => setConfirm(null)}>
              Cancel
            </button>
          </span>
        ) : (
          <button
            type="button"
            data-testid="prm-settings-reset-all"
            disabled={busy}
            onClick={() => setConfirm('all')}
          >
            Reset all
          </button>
        )}
        <button
          type="button"
          className="primary"
          data-testid="prm-settings-save"
          disabled={busy || !dirty || invalid}
          onClick={() => void save()}
        >
          Save
        </button>
      </div>
      {full && (
        <p className="prv-meta" data-testid="prm-settings-full">
          {TOO_MANY}
        </p>
      )}
      {dirty && !result && (
        <p className="prv-meta" data-testid="prm-settings-dirty">
          Unsaved changes.
        </p>
      )}
      {result && !result.key && (
        <p
          className={result.ok ? 'prv-meta' : 'prv-meta prv-error'}
          data-testid="prm-settings-result"
        >
          {result.text}
        </p>
      )}
    </div>
  );
}

interface RowProps {
  row: Row;
  first: boolean;
  last: boolean;
  /** The stored helper (absent for a new one). */
  stored: PromptHelper | undefined;
  busy: boolean;
  confirming: boolean;
  /** A save or reset error for this row. */
  error: string | null;
  onPatch(p: Partial<Row>): void;
  onMove(dir: -1 | 1): void;
  onReset(): void;
  onDelete(): void;
  onConfirmDelete(): void;
  onCancelDelete(): void;
}

function HelperRow(props: RowProps) {
  const { row: r, stored } = props;
  const problem = fieldError(r);
  const canReset =
    r.builtin && !!stored && (stored.modified || stored.hidden || rowDiffers(r, stored));
  const toggleKind = (k: ProjectKind) =>
    props.onPatch({
      kinds: KIND_TABS.map((t) => t.kind).filter((x) =>
        x === k ? !r.kinds.includes(k) : r.kinds.includes(x),
      ),
    });
  return (
    <li
      className="prm-settings-helper"
      data-testid="prm-settings-helper"
      data-record-id={r.key}
      data-hidden={r.hidden || undefined}
    >
      <div className="rgn-row">
        <input
          className="rgn-text"
          aria-label="Helper label"
          data-testid="prm-settings-helper-label"
          value={r.label}
          maxLength={60}
          placeholder="Label"
          aria-invalid={problem?.field === 'label' || undefined}
          onChange={(e) => props.onPatch({ label: e.target.value })}
        />
        <button
          type="button"
          className="prm-icon"
          aria-label="Move up"
          data-testid="prm-settings-helper-up"
          disabled={props.first}
          onClick={() => props.onMove(-1)}
        >
          ↑
        </button>
        <button
          type="button"
          className="prm-icon"
          aria-label="Move down"
          data-testid="prm-settings-helper-down"
          disabled={props.last}
          onClick={() => props.onMove(1)}
        >
          ↓
        </button>
      </div>
      <textarea
        className="rgn-prompt"
        aria-label="Helper text"
        data-testid="prm-settings-helper-text"
        rows={2}
        value={r.text}
        placeholder="Text added to the final prompt"
        aria-invalid={problem?.field === 'text' || undefined}
        onChange={(e) => props.onPatch({ text: e.target.value })}
      />
      <div className="prm-settings-flags">
        {KIND_TABS.map((t) => (
          <label key={t.kind} className="rgn-check">
            <input
              type="checkbox"
              data-testid="prm-settings-helper-kind"
              data-kind={t.kind}
              checked={r.kinds.includes(t.kind)}
              onChange={() => toggleKind(t.kind)}
            />
            {t.label}
          </label>
        ))}
        <label className="rgn-check">
          <input
            type="checkbox"
            data-testid="prm-settings-helper-default"
            checked={r.default_on}
            onChange={(e) => props.onPatch({ default_on: e.target.checked })}
          />
          On by default
        </label>
      </div>
      <div className="prm-settings-row-actions">
        {r.builtin ? (
          <>
            <button
              type="button"
              data-testid="prm-settings-helper-hide"
              aria-pressed={r.hidden}
              onClick={() => props.onPatch({ hidden: !r.hidden })}
            >
              {r.hidden ? 'Show' : 'Hide'}
            </button>
            <button
              type="button"
              data-testid="prm-settings-helper-reset"
              disabled={props.busy || !canReset}
              onClick={props.onReset}
            >
              Reset
            </button>
          </>
        ) : props.confirming ? (
          <span className="prm-confirm" data-testid="prm-settings-delete-ask">
            Delete this helper?
            <button
              type="button"
              className="prm-danger"
              data-testid="prm-settings-delete-confirm"
              onClick={props.onConfirmDelete}
            >
              Delete
            </button>
            <button type="button" onClick={props.onCancelDelete}>
              Cancel
            </button>
          </span>
        ) : (
          <button type="button" data-testid="prm-settings-helper-delete" onClick={props.onDelete}>
            Delete
          </button>
        )}
        {r.hidden && <span className="pf-chip">Hidden</span>}
      </div>
      {(problem || props.error) && (
        <p className="prv-meta prv-error" data-testid="prm-settings-helper-error">
          {props.error ?? problem?.message}
        </p>
      )}
    </li>
  );
}
