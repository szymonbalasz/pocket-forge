// The live mask (FR-EDT-14..16, 19..22): a region-sized 8-bit buffer, mirrored into a CPU-backed
// canvas (`willReadFrequently`, so it survives GPU resets) shown as a 50 % tint, plus per-stroke
// undo entries holding the before/after alpha of the stroke's bounding box. A stroke saves each
// mask row the first time a dab touches it, not the whole mask at pointer-down (BL-09).
import {
  dabBox,
  dabsAlong,
  dabSpacing,
  emptyMask,
  extract,
  remap,
  restore,
  stamp,
  unionBox,
  type Box,
  type MaskBuffer,
  type StampMode,
} from './mask';
import type { Rect, RegionState } from './store';
import type { UndoEntry, UndoStack } from './undo';
import type { Pt } from './view';

/** Where the mask is shown. The canvas implementation is in EditorCanvas; tests use a fake. */
export interface MaskView {
  /** A new buffer (size or position changed), or null when the region is removed. */
  reset(m: MaskBuffer | null): void;
  /** Pixels in `box` (mask px) changed. */
  update(m: MaskBuffer, box: Box): void;
}

interface Stroke {
  mode: StampMode;
  r: number;
  hardness: number;
  last: Pt;
  carry: number;
  box: Box | null;
  /** Rows as they were before the stroke first touched them (full mask width). */
  before: (Uint8Array | undefined)[];
}

export interface MaskControllerDeps {
  undo: UndoStack;
  view: MaskView;
  /** Sets the region in the editor state (no fit: the stored validated region). */
  setRegion: (r: RegionState | null) => void;
  /** Called after every change of mask content and at every stroke end (debounced submit). */
  onChange?: () => void;
}

const sameRect = (a: Rect, b: Rect) => a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
const sameRegion = (a: RegionState | null, b: RegionState | null) =>
  a === b || (!!a && !!b && sameRect(a.inner, b.inner) && sameRect(a.outer, b.outer));

export class MaskController {
  private mask: MaskBuffer | null = null;
  private region: RegionState | null = null;
  private stroke: Stroke | null = null;
  /** This canvas has pushed an undo entry (the DM changed something here). */
  private edited = false;
  /** Bumped on every content change; slice 5 compares it with the last submitted version. */
  version = 0;

  constructor(private readonly deps: MaskControllerDeps) {}

  /** Draws the current mask on a new view: the Stage remounted with a new mask layer (D1). */
  setView(view: MaskView): void {
    this.deps.view = view;
    view.reset(this.mask);
  }

  get buffer(): MaskBuffer | null {
    return this.mask;
  }

  get currentRegion(): RegionState | null {
    return this.region;
  }

  /** A stroke is in progress (the debounced submit waits for its end, FR-EDT-28). */
  get stroking(): boolean {
    return this.stroke !== null;
  }

  /** Sets the region without an undo step (open, reopen). */
  load(region: RegionState | null, mask: MaskBuffer | null): void {
    this.region = region;
    this.mask = region ? (mask ? remap(mask, region.inner) : emptyMask(region.inner)) : null;
    this.deps.setRegion(region);
    this.deps.view.reset(this.mask);
    this.changed();
  }

  /** FR-EDT-20: a region change keeps the overlap and discards the rest; removing the region
   *  discards the mask. One undo step. */
  changeRegion(next: RegionState | null): void {
    const prev = { region: this.region, mask: this.mask };
    // The same rect (a Region-tool click inside the box): no change, no step, Redo stays.
    if (sameRegion(prev.region, next)) return;
    const nextMask = next
      ? this.mask
        ? remap(this.mask, next.inner)
        : emptyMask(next.inner)
      : null;
    const apply = (region: RegionState | null, mask: MaskBuffer | null) => {
      this.region = region;
      this.mask = mask;
      this.deps.setRegion(region);
      this.deps.view.reset(mask);
      this.changed();
    };
    apply(next, nextMask);
    this.edited = true;
    this.deps.undo.push({
      label: next ? (prev.region ? 'region change' : 'new region') : 'remove region',
      bytes: (prev.mask?.data.length ?? 0) + (nextMask?.data.length ?? 0),
      undo: () => apply(prev.region, prev.mask),
      redo: () => apply(next, nextMask),
      canvas: true,
    });
  }

  /** BL-14: another feature's validated region (RGN's fit after a margin change, a model change
   *  or Retry). Same inner rect → only the outline follows (no undo step, mask untouched); a new
   *  inner rect is a region change (FR-EDT-20: overlap kept, one undo step). */
  applyValidated(next: RegionState | null): void {
    const cur = this.region;
    if (!next) {
      if (cur) this.changeRegion(null);
      return;
    }
    // Walk-gen L1: on a fresh canvas (a remount for a new bottom image, before its draft load)
    // RGN's echo of its region is not an edit: no undo step, which would also clear Redo.
    if (!cur && !this.edited) {
      this.load({ inner: { ...next.inner }, outer: { ...next.outer } }, null);
      return;
    }
    if (cur && sameRect(cur.inner, next.inner)) {
      if (sameRect(cur.outer, next.outer)) return;
      this.region = { inner: { ...next.inner }, outer: { ...next.outer } };
      this.deps.setRegion(this.region);
      return;
    }
    this.changeRegion({ inner: { ...next.inner }, outer: { ...next.outer } });
  }

  /** FR-EDT-15: starts a stroke at image point p. False when there is no region (FR-EDT-17). */
  beginStroke(p: Pt, mode: StampMode, size: number, hardness: number): boolean {
    if (!this.mask) return false;
    this.stroke = {
      mode,
      r: size / 2,
      hardness,
      last: p,
      carry: 0,
      box: null,
      before: [],
    };
    this.dab(p);
    return true;
  }

  moveStroke(p: Pt): void {
    const s = this.stroke;
    if (!s) return;
    const { dabs, carry } = dabsAlong(s.last, p, dabSpacing(s.r), s.carry);
    for (const d of dabs) this.dab(d);
    s.carry = carry;
    s.last = p;
  }

  /** Ends the stroke: one undo entry with the before/after alpha of its bounding box. */
  endStroke(): void {
    const s = this.stroke;
    const m = this.mask;
    this.stroke = null;
    // The submit debounce counts from the stroke end (R7).
    if (s) this.deps.onChange?.();
    if (!s || !m || !s.box) return;
    const box = s.box;
    const w = m.rect.w;
    const after = extract(m.data, w, box);
    // Rows the stroke never touched are unchanged, so the current bytes are their "before".
    const before = after.slice();
    const bw = box.x1 - box.x0;
    for (let y = box.y0; y < box.y1; y++) {
      const row = s.before[y];
      if (row) before.set(row.subarray(box.x0, box.x1), (y - box.y0) * bw);
    }
    this.boxEntry(s.mode === 'paint' ? 'brush stroke' : 'eraser stroke', m, box, before, after);
  }

  /** FR-EDT-16: Mask whole region (255, owner 2026-11-08) / Clear mask (0). One undo step each. */
  fillAll(value: 0 | 255): void {
    const m = this.mask;
    if (!m) return;
    const box = { x0: 0, y0: 0, x1: m.rect.w, y1: m.rect.h };
    const before = m.data.slice();
    m.data.fill(value);
    const after = m.data.slice();
    this.deps.view.update(m, box);
    this.changed();
    this.boxEntry(value ? 'mask whole region' : 'clear mask', m, box, before, after);
  }

  private boxEntry(label: string, m: MaskBuffer, box: Box, before: Uint8Array, after: Uint8Array) {
    const set = (src: Uint8Array) => {
      // The entry belongs to this buffer; after a region change it is replaced, and the region
      // entry restores the buffer first, so `this.mask === m` whenever this runs in order.
      if (this.mask !== m) return;
      restore(m.data, m.rect.w, box, src);
      this.deps.view.update(m, box);
      this.changed();
    };
    const entry: UndoEntry = {
      label,
      bytes: before.length + after.length,
      undo: () => set(before),
      redo: () => set(after),
      canvas: true,
    };
    this.edited = true;
    this.deps.undo.push(entry);
  }

  private dab(p: Pt): void {
    const s = this.stroke!;
    const m = this.mask!;
    const cx = p.x - m.rect.x;
    const cy = p.y - m.rect.y;
    const touched = dabBox(m, cx, cy, s.r);
    if (!touched) return;
    const w = m.rect.w;
    for (let y = touched.y0; y < touched.y1; y++) s.before[y] ??= m.data.slice(y * w, (y + 1) * w);
    const b = stamp(m, cx, cy, s.r, s.hardness, s.mode);
    if (!b) return;
    s.box = unionBox(s.box, b);
    this.deps.view.update(m, b);
    this.changed();
  }

  private changed(): void {
    this.version++;
    this.deps.onChange?.();
  }
}
