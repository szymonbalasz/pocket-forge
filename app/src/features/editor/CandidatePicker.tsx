// The candidate picker (FR-EDT-25, R19): a strip of the candidates it is given, with thumbnail,
// index, UI status and star. Click, 1–9 or ←/→ select a ready one (onSelect); Enter or Accept
// accepts (onAccept); star and discard emit onStar / onDiscard. It calls no core command.
import { useEffect } from 'react';

import { IconCheck, IconCompare, IconStar, IconTrash } from '../../shell/icons';
import { dispatchShortcut, noMouseFocus, type ShortcutMap } from './shortcuts';
import { editorStore, useEditor, type PickerCandidate } from './store';
import { Tip } from './Tip';

/** Selects candidate `id` if it is ready: updates the compare source and emits onSelect. */
export function selectCandidate(id: string): void {
  const p = editorStore.get().picker;
  const c = p?.candidates.find((x) => x.id === id);
  if (!p || !c || c.status !== 'ready') return;
  editorStore.set({ picker: { ...p, selectedId: id }, compare: c.preview ?? null });
  p.handlers.onSelect?.(id);
}

function step(dir: 1 | -1): void {
  const p = editorStore.get().picker;
  if (!p) return;
  const ready = p.candidates.filter((c) => c.status === 'ready');
  if (!ready.length) return;
  const i = ready.findIndex((c) => c.id === p.selectedId);
  const next = i < 0 ? (dir > 0 ? 0 : ready.length - 1) : (i + dir + ready.length) % ready.length;
  selectCandidate(ready[next]!.id);
}

function accept(): void {
  const p = editorStore.get().picker;
  if (p?.selectedId) p.handlers.onAccept?.(p.selectedId);
}

const COMPARE_TIP = 'Compare mode: split or flip (C). In flip, hold \\ to see before.';

export function EditorPicker() {
  const picker = useEditor((s) => s.picker);
  const mode = useEditor((s) => s.compareMode);
  const active = !!picker;

  useEffect(() => {
    if (!active) return;
    const map: ShortcutMap = {
      ArrowLeft: () => step(-1),
      ArrowRight: () => step(1),
      Enter: () => accept(),
    };
    for (let n = 1; n <= 9; n++)
      map[String(n)] = () => {
        const c = editorStore.get().picker?.candidates[n - 1];
        if (c) selectCandidate(c.id);
      };
    const down = (e: KeyboardEvent) => void dispatchShortcut(e, map);
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [active]);

  if (!picker || picker.candidates.length === 0) return null;
  const { candidates, selectedId, handlers } = picker;
  return (
    <div
      className="edt-picker"
      role="listbox"
      onMouseDown={noMouseFocus}
      aria-label="Candidates"
      data-testid="edt-picker"
    >
      {candidates.map((c: PickerCandidate, i) => (
        <div
          key={c.id}
          role="option"
          aria-selected={c.id === selectedId}
          className={`edt-picker-item edt-picker-${c.status}`}
          data-testid="edt-picker-item"
          data-record-id={c.id}
          data-status={c.status}
          onClick={() => selectCandidate(c.id)}
        >
          <div className="edt-picker-thumb">
            {c.status === 'ready' && c.thumbUrl ? (
              <img src={c.thumbUrl} alt={`Candidate ${c.index}`} draggable={false} />
            ) : (
              <span className="muted">{c.status === 'pending' ? '…' : 'Invalid'}</span>
            )}
            {c.warning && (
              <span
                className="edt-picker-warning"
                data-testid="edt-picker-warning"
                role="img"
                aria-label={c.warning}
                title={c.warning}
              >
                !
              </span>
            )}
          </div>
          <div className="edt-picker-meta">
            <span className="tabular" title={i < 9 ? `Key ${i + 1}` : undefined}>
              {c.index}
            </span>
            <button
              type="button"
              className="edt-tool edt-picker-btn"
              data-testid="edt-picker-star"
              aria-pressed={c.starred}
              aria-label={c.starred ? 'Unstar' : 'Star'}
              disabled={c.status === 'pending'}
              onClick={(e) => {
                e.stopPropagation();
                handlers.onStar?.(c.id, !c.starred);
              }}
            >
              <IconStar size={14} fill={c.starred ? 'currentColor' : 'none'} />
            </button>
            <button
              type="button"
              className="edt-tool edt-picker-btn"
              data-testid="edt-picker-discard"
              aria-label="Discard"
              disabled={c.status === 'pending'}
              onClick={(e) => {
                e.stopPropagation();
                handlers.onDiscard?.(c.id);
              }}
            >
              <IconTrash size={14} />
            </button>
          </div>
        </div>
      ))}
      <span className="pf-spacer" />
      <Tip text={COMPARE_TIP} side="top">
        <button
          type="button"
          data-testid="edt-compare-mode"
          data-mode={mode}
          onClick={() =>
            editorStore.set((s) => ({ compareMode: s.compareMode === 'split' ? 'flip' : 'split' }))
          }
        >
          <IconCompare size={14} /> {mode === 'split' ? 'Split' : 'Flip'}
        </button>
      </Tip>
      <Tip text="Accept the selected candidate (Enter)" side="top">
        <button
          type="button"
          className="primary"
          data-testid="edt-picker-accept"
          disabled={!selectedId}
          onClick={accept}
        >
          <IconCheck size={14} /> Accept
        </button>
      </Tip>
    </div>
  );
}
