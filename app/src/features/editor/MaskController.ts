// The live mask (FR-EDT-14..16, 19..22): a region-sized 8-bit buffer, mirrored into a CPU-backed
// canvas (`willReadFrequently`, so it survives GPU resets) shown as a 50 % tint, plus per-stroke
// undo entries holding the before/after alpha of the stroke's bounding box.
import {
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
import type { RegionState } from './store';
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
  before: Uint8Array;
}

export interface MaskControllerDeps {
  undo: UndoStack;
  view: MaskView;
  /** Sets the region in the editor state (no fit: the stored validated region). */
  setRegion: (r: RegionState | null) => void;
  /** Called after every change of mask content (debounced submit, slice 5). */
  onChange?: () => void;
}

export class MaskController {
  private mask: MaskBuffer | null = null;
  private region: RegionState | null = null;
  private stroke: Stroke | null = null;
  /** Bumped on every content change; slice 5 compares it with the last submitted version. */
  version = 0;

  constructor(private readonly deps: MaskControllerDeps) {}

  get buffer(): MaskBuffer | null {
    return this.mask;
  }

  get currentRegion(): RegionState | null {
    return this.region;
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
    this.deps.undo.push({
      label: next ? (prev.region ? 'region change' : 'new region') : 'remove region',
      bytes: (prev.mask?.data.length ?? 0) + (nextMask?.data.length ?? 0),
      undo: () => apply(prev.region, prev.mask),
      redo: () => apply(next, nextMask),
    });
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
      before: this.mask.data.slice(),
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
    if (!s || !m || !s.box) return;
    const box = s.box;
    const w = m.rect.w;
    const before = extract(s.before, w, box);
    const after = extract(m.data, w, box);
    this.boxEntry(s.mode === 'paint' ? 'brush stroke' : 'eraser stroke', m, box, before, after);
  }

  /** FR-EDT-16: Fill region (255) / Clear mask (0). One undo step each. */
  fillAll(value: 0 | 255): void {
    const m = this.mask;
    if (!m) return;
    const box = { x0: 0, y0: 0, x1: m.rect.w, y1: m.rect.h };
    const before = m.data.slice();
    m.data.fill(value);
    const after = m.data.slice();
    this.deps.view.update(m, box);
    this.changed();
    this.boxEntry(value ? 'fill region' : 'clear mask', m, box, before, after);
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
    };
    this.deps.undo.push(entry);
  }

  private dab(p: Pt): void {
    const s = this.stroke!;
    const m = this.mask!;
    const b = stamp(m, p.x - m.rect.x, p.y - m.rect.y, s.r, s.hardness, s.mode);
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
