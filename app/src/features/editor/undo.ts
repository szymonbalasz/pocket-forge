// The session undo stack (FR-EDT-21..23, KQ-16): one stack shared by every feature. Mask strokes
// push alpha snapshots; other features push entries whose undo/redo call core commands (accept and
// re-tune undo through `project_restore_revision`, R3). Not persisted; cleared on close.

export interface UndoEntry {
  label: string;
  /** Memory the entry holds (alpha bytes for mask entries, ~0 for command entries). */
  bytes: number;
  undo(): void | Promise<void>;
  redo(): void | Promise<void>;
  /** Tied to one canvas instance (mask and region snapshots): dropped when the canvas remounts
   *  for a new bottom image (a pick or its undo, FR-GEN-29). Command entries survive it. */
  canvas?: boolean;
}

export interface UndoSnapshot {
  depth: number;
  redo: number;
  label: string | null;
  busy: boolean;
  error: string | null;
  bytes: number;
}

/** FR-EDT-22: 256 MiB of undo memory (alpha only). */
export const UNDO_BYTES = 256 * 1024 * 1024;

function errorCode(e: unknown): string {
  if (e && typeof e === 'object' && 'code' in e) return String((e as { code: unknown }).code);
  return 'ui';
}

export class UndoStack {
  private readonly done: UndoEntry[] = [];
  private readonly undone: UndoEntry[] = [];
  private total = 0;
  private busy = false;
  private error: string | null = null;
  /** Bumped by every push (a new action) and every clear (a project switch), so a step in
   *  progress can tell that the stack moved on while its call ran (FR-EDT-21/23). */
  private pushes = 0;
  private epoch = 0;
  private readonly listeners = new Set<(s: UndoSnapshot) => void>();

  constructor(readonly budget = UNDO_BYTES) {}

  subscribe(l: (s: UndoSnapshot) => void): () => void {
    this.listeners.add(l);
    l(this.snapshot());
    return () => this.listeners.delete(l);
  }

  snapshot(): UndoSnapshot {
    return {
      depth: this.done.length,
      redo: this.undone.length,
      label: this.done.at(-1)?.label ?? null,
      busy: this.busy,
      error: this.error,
      bytes: this.total,
    };
  }

  /** A new action: clears redo; drops the oldest entries over budget but keeps the newest. */
  push(e: UndoEntry): void {
    for (const u of this.undone) this.total -= u.bytes;
    // In place: a step in progress holds these arrays (walk-gen L1).
    this.undone.length = 0;
    this.done.push(e);
    this.pushes += 1;
    this.total += e.bytes;
    while (this.total > this.budget && this.done.length > 1) {
      this.total -= this.done.shift()!.bytes;
    }
    this.error = null;
    this.emit();
  }

  /** FR-EDT-23: if the entry's call fails, the position doesn't move and the code is kept. */
  async undo(): Promise<boolean> {
    return this.step(this.done, this.undone, 'undo');
  }

  async redo(): Promise<boolean> {
    return this.step(this.undone, this.done, 'redo');
  }

  /** The canvas remounted within the same project: its mask entries can't apply any more. */
  dropCanvasEntries(): void {
    // In place: a step in progress (an undo whose reload remounts the canvas) holds these arrays.
    for (const list of [this.done, this.undone]) {
      const kept = list.filter((e) => !e.canvas);
      list.splice(0, list.length, ...kept);
    }
    this.total = [...this.done, ...this.undone].reduce((n, e) => n + e.bytes, 0);
    this.emit();
  }

  clear(): void {
    this.done.length = 0;
    this.undone.length = 0;
    this.epoch += 1;
    this.total = 0;
    this.error = null;
    this.busy = false;
    this.emit();
  }

  private async step(from: UndoEntry[], to: UndoEntry[], op: 'undo' | 'redo'): Promise<boolean> {
    const e = from.at(-1);
    if (!e || this.busy) return false;
    this.busy = true;
    this.error = null;
    this.emit();
    const epoch = this.epoch;
    const pushes = this.pushes;
    try {
      await e[op]();
      // The stack was cleared (another project opened): this entry belongs to none of it.
      if (this.epoch !== epoch) return false;
      // The exact entry: a push or a canvas drop may have changed the stacks meanwhile.
      const i = from.lastIndexOf(e);
      if (i >= 0) from.splice(i, 1);
      const newer = this.pushes - pushes;
      if (newer === 0) {
        if (i >= 0) to.push(e);
        // else a canvas drop or the budget removed it: it stays gone.
      } else if (op === 'undo') {
        // A new action came in while the undo ran: it clears redo (FR-EDT-21), this entry too.
        if (i >= 0) this.total -= e.bytes;
      } else {
        // The redo is applied, then the new actions: it goes under them (the push already
        // cleared it from redo and uncounted it).
        this.done.splice(Math.max(0, this.done.length - newer), 0, e);
        if (i < 0) this.total += e.bytes;
      }
      return true;
    } catch (err) {
      this.error = errorCode(err);
      return false;
    } finally {
      this.busy = false;
      this.emit();
    }
  }

  private emit(): void {
    const s = this.snapshot();
    for (const l of this.listeners) l(s);
  }
}
