// The candidate picker (FR-EDT-25, R19): a strip of the candidates it is given, with thumbnail,
// index, UI status and star. Click, 1–9 or ←/→ select a ready one (onSelect); Enter or Accept
// accepts (onAccept); star and discard emit onStar / onDiscard. #56: X discards the selected
// candidate, Delete or X the focused one, and the selection moves on; "Discard the rest" emits
// onDiscardRest. It calls no core command. #63: a ready item's context menu (EXP's) saves,
// exports or reveals its file.
import { useEffect, type KeyboardEvent as ReactKeyboardEvent, type ReactElement } from 'react';

import { IconCheck, IconCompare, IconStar, IconTrash } from '../../shell/icons';
import { FileMenu } from '../export';
import { dispatchShortcut, noMouseFocus, type ShortcutMap } from './shortcuts';
import { editorStore, useEditor, type PickerCandidate } from './store';
import { Tip } from './Tip';

/** #63: Save as…, Export as ▸ and Open in Explorer on a ready candidate with a file
 *  (`edt-picker-menu`, FR-EXP-32). */
function ItemMenu({ c, children }: { c: PickerCandidate; children: ReactElement }) {
  if (!c.file || c.status !== 'ready') return children;
  return (
    <FileMenu testId="edt-picker-menu" path={c.file} image>
      {children}
    </FileMenu>
  );
}

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

const itemEl = (id: string) =>
  document.querySelector<HTMLElement>(`[data-testid="edt-picker-item"][data-record-id="${id}"]`);

/** #56: emits onDiscard for `id`; when it was selected (or `focus`), the next ready candidate
 *  (else the previous one) takes the selection, and keyboard focus too when `focus`. */
export function discardCandidate(id: string, focus = false): void {
  const p = editorStore.get().picker;
  const c = p?.candidates.find((x) => x.id === id);
  if (!p || !c || c.status === 'pending' || c.accepted) return;
  const i = p.candidates.indexOf(c);
  const ready = (x: PickerCandidate) => x.status === 'ready' && x.id !== id;
  const next =
    p.candidates.slice(i + 1).find(ready) ?? p.candidates.slice(0, i).reverse().find(ready);
  p.handlers.onDiscard?.(id);
  if (!next) return;
  if (p.selectedId === id || focus) selectCandidate(next.id);
  if (focus) itemEl(next.id)?.focus();
}

/** Delete or X on a focused item (no modifiers) discards it; the window's Delete (remove the
 *  region) and X don't also run. */
function onItemKey(e: ReactKeyboardEvent, id: string): void {
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key !== 'Delete' && e.key.toLowerCase() !== 'x') return;
  e.preventDefault();
  e.stopPropagation();
  discardCandidate(id, true);
}

function accept(): void {
  const p = editorStore.get().picker;
  if (p?.selectedId) p.handlers.onAccept?.(p.selectedId);
}

const DISCARD_REST_TIP =
  'Discard every candidate except the accepted and starred ones (to the trash for 30 days).';
const COMPARE_TIP = 'Compare mode: split or flip (C). In flip, hold \\ to see before.';

export function EditorPicker() {
  const picker = useEditor((s) => s.picker);
  const mode = useEditor((s) => s.compareMode);
  // FR-GEN-26: a candidate shown alone has no split or flip (walk-gen L7).
  const alone = useEditor((s) => !!s.compare?.alone);
  const active = !!picker;

  useEffect(() => {
    if (!active) return;
    const map: ShortcutMap = {
      ArrowLeft: () => step(-1),
      ArrowRight: () => step(1),
      Enter: () => accept(),
      X: () => {
        const id = editorStore.get().picker?.selectedId;
        if (id) discardCandidate(id);
      },
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
        <ItemMenu key={c.id} c={c}>
          <div
            role="option"
            aria-selected={c.id === selectedId}
            className={`edt-picker-item edt-picker-${c.status}`}
            data-testid="edt-picker-item"
            data-record-id={c.id}
            data-status={c.status}
            data-accepted={c.accepted ? 'true' : undefined}
            tabIndex={c.status === 'pending' ? -1 : 0}
            onClick={() => selectCandidate(c.id)}
            onKeyDown={(e) => onItemKey(e, c.id)}
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
              {/* Walk owner-UX L4: a Radix tip, not a native title (ui-guidelines). */}
              <Tip
                side="top"
                text={
                  c.accepted
                    ? 'The accepted candidate can’t be discarded'
                    : 'Discard: to the trash for 30 days (X, or Delete on a focused candidate)'
                }
              >
                <button
                  type="button"
                  className="edt-tool edt-picker-btn"
                  data-testid="edt-picker-discard"
                  aria-label="Discard"
                  disabled={c.status === 'pending' || c.accepted}
                  onClick={(e) => {
                    e.stopPropagation();
                    discardCandidate(c.id);
                  }}
                >
                  <IconTrash size={14} />
                </button>
              </Tip>
            </div>
          </div>
        </ItemMenu>
      ))}
      <span className="pf-spacer" />
      {handlers.onDiscardRest && (
        <Tip text={DISCARD_REST_TIP} side="top">
          <button
            type="button"
            className="edt-picker-discard-rest"
            data-testid="edt-picker-discard-rest"
            onClick={() => handlers.onDiscardRest?.()}
          >
            <IconTrash size={14} /> Discard the rest
          </button>
        </Tip>
      )}
      {!alone && (
        <Tip text={COMPARE_TIP} side="top">
          <button
            type="button"
            data-testid="edt-compare-mode"
            data-mode={mode}
            onClick={() =>
              editorStore.set((s) => ({
                compareMode: s.compareMode === 'split' ? 'flip' : 'split',
              }))
            }
          >
            <IconCompare size={14} /> {mode === 'split' ? 'Split' : 'Flip'}
          </button>
        </Tip>
      )}
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
