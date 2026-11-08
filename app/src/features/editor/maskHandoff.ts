// EDT slice 5 (FR-EDT-18, 28, 29; R7): hands the live mask to PRJ's store (`put_mask`) through
// `edt_mask_submit`. Triggers: ≥ 2 s after the last stroke end (debounced), before a job submit
// (`flush`, RGN) and before the project closes (`flush`, PRJ's close guard). A submit is skipped when
// the bytes and rect equal the last successful submit (or the mask loaded on reopen). A read-only
// project never submits (FR-EDT-29). `locked` / `conflict` / `io` keep the mask in memory, show
// "unsaved" and retry at the next trigger or the next commit (EDT §11).
import { api, ApiError, type MaskRef } from '../../api';
import type { MaskBuffer } from './mask';
import type { Rect } from './store';

export type UnsavedReason = 'pending' | 'read_only' | 'locked' | 'conflict' | 'io' | 'error';

export type MaskStatus =
  | { kind: 'none' }
  | { kind: 'saved' }
  | { kind: 'saving' }
  | { kind: 'unsaved'; reason: UnsavedReason; message?: string };

/** Debounce after the last stroke end before submitting (R7, ≥ 2 s). */
export const MASK_SUBMIT_DEBOUNCE_MS = 2000;

export interface MaskHandoff {
  /** Submits the mask's w×h bytes for its inner rect; resolves to the stored mask's ref. */
  submit(projectId: string, mask: MaskBuffer): Promise<MaskRef>;
}

/** The real hand-off: `edt_mask_submit` (bytes as base64 over IPC). */
export const coreHandoff: MaskHandoff = {
  submit: (projectId, mask) => api.mask.submit(projectId, mask.rect, mask.data),
};

export interface MaskSyncDeps {
  projectId: string;
  handoff: MaskHandoff;
  /** The live mask (null = no region). */
  buffer: () => MaskBuffer | null;
  /** Bumped on every content change (MaskController.version). */
  version: () => number;
  /** A stroke is in progress: never submit mid-stroke. */
  stroking: () => boolean;
  readOnly: () => boolean;
  onStatus: (s: MaskStatus) => void;
  debounceMs?: number;
}

const sameRect = (a: Rect, b: Rect) => a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;

function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

const allZero = (d: Uint8Array) => d.every((v) => v === 0);

function reasonOf(e: unknown): { reason: UnsavedReason; message: string } {
  const code = e instanceof ApiError ? e.code : 'error';
  const message = e instanceof Error ? e.message : String(e);
  const reason: UnsavedReason =
    code === 'read_only' || code === 'locked' || code === 'conflict' || code === 'io'
      ? code
      : 'error';
  return { reason, message };
}

/** The mask's save state and its submit triggers, for one open project. */
export class MaskSync {
  /** What the store holds: the last submitted (or reopened) rect + bytes, and its ref. */
  private saved: { rect: Rect; data: Uint8Array; ref: MaskRef | null } | null = null;
  private savedVersion = -1;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inFlight: Promise<MaskRef | null> | null = null;
  private failed = false;
  private disposed = false;
  private status: MaskStatus = { kind: 'none' };

  constructor(private readonly deps: MaskSyncDeps) {}

  /** The mask loaded on reopen is what the store holds: no submit until it changes (FR-EDT-19). */
  markLoaded(mask: MaskBuffer | null, maskId: string | null): void {
    this.saved = mask
      ? {
          rect: { ...mask.rect },
          data: mask.data.slice(),
          ref: maskId ? { mask_id: maskId, rect: { ...mask.rect } } : null,
        }
      : null;
    this.refresh();
  }

  /** Any content change (dabs, fill, clear, region, undo, stroke end): restart the debounce.
   *  Called per dab, so O(1): the exact comparison waits for the timer or a flush. */
  changed(): void {
    if (this.disposed) return;
    this.refresh(false);
    if (!this.deps.buffer() || this.deps.readOnly()) return;
    if (this.deps.version() === this.savedVersion) return;
    this.schedule();
  }

  /** Retries a failed submit (after a commit, e.g. a resolved conflict). */
  retry(): void {
    if (this.failed && !this.isClean()) this.schedule();
  }

  /** Submits now if the mask changed (job submit, close). Resolves to the ref the draft points
   *  at, or null when there is no mask or the project is read-only. Rejects on a failed submit. */
  async flush(): Promise<MaskRef | null> {
    this.clearTimer();
    if (this.inFlight) await this.inFlight.catch(() => undefined);
    if (this.disposed || this.deps.readOnly() || !this.deps.buffer()) {
      this.refresh();
      return null;
    }
    if (this.isClean()) return this.saved?.ref ?? null;
    return this.run(true);
  }

  dispose(): void {
    this.disposed = true;
    this.clearTimer();
  }

  get current(): MaskStatus {
    return this.status;
  }

  /** True if `flush` may have to submit or wait (cheap: versions only). */
  get pending(): boolean {
    if (this.disposed || this.deps.readOnly() || !this.deps.buffer()) return false;
    return this.inFlight !== null || this.timer !== null || !this.isClean(false);
  }

  private schedule(): void {
    this.clearTimer();
    this.timer = setTimeout(() => {
      this.timer = null;
      if (this.deps.stroking()) {
        this.schedule();
        return;
      }
      // Undo or clear-then-fill can bring the mask back to what's stored.
      if (this.isClean()) {
        this.refresh();
        return;
      }
      void this.run(false).catch(() => undefined);
    }, this.deps.debounceMs ?? MASK_SUBMIT_DEBOUNCE_MS);
  }

  private clearTimer(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  /** True when the live mask equals what the store holds (or there is nothing to save).
   *  `exact: false` only compares versions (cheap; a changed version counts as dirty). */
  private isClean(exact = true): boolean {
    const m = this.deps.buffer();
    if (!m) return true;
    const v = this.deps.version();
    if (v === this.savedVersion) return true;
    if (!exact) return false;
    const clean = this.saved
      ? sameRect(this.saved.rect, m.rect) && sameBytes(this.saved.data, m.data)
      : allZero(m.data); // An empty new mask isn't worth a file.
    if (clean) this.savedVersion = v;
    return clean;
  }

  private refresh(exact = true): void {
    const m = this.deps.buffer();
    let next: MaskStatus;
    if (!m) next = { kind: 'none' };
    else if (this.deps.readOnly()) next = unsaved('read_only');
    else if (this.inFlight) next = { kind: 'saving' };
    else if (this.isClean(exact)) next = this.saved ? { kind: 'saved' } : { kind: 'none' };
    else if (this.failed && this.status.kind === 'unsaved') next = this.status;
    else next = unsaved('pending');
    this.setStatus(next);
  }

  private setStatus(s: MaskStatus): void {
    const a = this.status;
    if (
      a.kind === s.kind &&
      (a.kind !== 'unsaved' ||
        (s.kind === 'unsaved' && a.reason === s.reason && a.message === s.message))
    )
      return;
    this.status = s;
    this.deps.onStatus(s);
  }

  private run(rethrow: boolean): Promise<MaskRef | null> {
    const m = this.deps.buffer();
    if (this.disposed || !m || this.deps.readOnly()) {
      this.refresh();
      return Promise.resolve(null);
    }
    const version = this.deps.version();
    const rect = { ...m.rect };
    const data = m.data.slice(); // The bytes sent are the bytes remembered (FR-EDT-18).
    // Started on a microtask, so `p` is assigned before the body can reach `finally`.
    const p: Promise<MaskRef | null> = Promise.resolve().then(async () => {
      try {
        const ref = await this.deps.handoff.submit(this.deps.projectId, { rect, data });
        this.saved = { rect, data, ref };
        this.savedVersion = version;
        this.failed = false;
        return ref;
      } catch (e) {
        this.failed = true;
        const { reason, message } = reasonOf(e);
        if (this.inFlight === p) this.inFlight = null;
        this.setStatus(unsaved(reason, message));
        throw e;
      } finally {
        if (this.inFlight === p) this.inFlight = null;
        if (!this.disposed) {
          if (!this.failed) this.refresh();
          // Changed while saving: submit again after the debounce.
          if (!this.failed && !this.isClean()) this.schedule();
        }
      }
    });
    this.inFlight = p;
    this.setStatus({ kind: 'saving' });
    return rethrow ? p : p.catch(() => null);
  }
}

function unsaved(reason: UnsavedReason, message?: string): MaskStatus {
  return message === undefined ? { kind: 'unsaved', reason } : { kind: 'unsaved', reason, message };
}

/** The status-bar text (`edt-mask-status`). */
export function maskStatusText(s: MaskStatus): string {
  switch (s.kind) {
    case 'none':
      return '';
    case 'saved':
      return 'Mask saved';
    case 'saving':
      return 'Saving mask…';
    case 'unsaved':
      switch (s.reason) {
        case 'pending':
          return 'Mask unsaved';
        case 'read_only':
          return 'Read-only: mask not saved';
        case 'locked':
          return 'Mask unsaved (file locked, will retry)';
        case 'conflict':
          return 'Mask unsaved (sync conflict)';
        default:
          return `Mask unsaved (${s.message ?? 'could not write'})`;
      }
  }
}
