// Session-wide editor objects other features use through the editor's index.ts: the shared undo
// stack (FR-EDT-23) and the revision-restore entry used to undo accept and re-tune (R3).
import { api } from '../../api';
import { UndoStack, type UndoEntry } from './undo';

/** The one undo stack of the editing session (KQ-16). Cleared when the project closes. */
export const undoStack = new UndoStack();

/** Pushes another feature's entry `{label, bytes, undo(), redo()}` (FR-EDT-23). */
export const pushUndo = (e: UndoEntry): void => undoStack.push(e);

/** FR-EDT-23 / R3: undo = restore the parent revision, redo = restore the later one. */
export function revisionUndoEntry(
  label: string,
  parentRevId: string,
  laterRevId: string,
): UndoEntry {
  return {
    label,
    bytes: 0,
    undo: async () => {
      await api.project.restoreRevision(parentRevId);
    },
    redo: async () => {
      await api.project.restoreRevision(laterRevId);
    },
  };
}
