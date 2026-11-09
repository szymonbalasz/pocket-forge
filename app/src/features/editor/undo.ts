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
  private done: UndoEntry[] = [];
  private undone: UndoEntry[] = [];
  private total = 0;
  private busy = false;
  private error: string | null = null;
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
    this.undone = [];
    this.done.push(e);
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
    this.done = [];
    this.undone = [];
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
    try {
      await e[op]();
      from.pop();
      to.push(e);
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
