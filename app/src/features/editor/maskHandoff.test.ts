import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { ApiError, fromBase64, toBase64, type MaskRef } from '../../api';
import { installTestCore } from '../../api/mocks/testCore';
import { emptyMask, type MaskBuffer } from './mask';
import { MaskController, type MaskView } from './MaskController';
import {
  coreHandoff,
  MASK_SUBMIT_DEBOUNCE_MS,
  maskStatusText,
  MaskSync,
  type MaskHandoff,
  type MaskStatus,
} from './maskHandoff';
import type { RegionState } from './store';
import { UndoStack } from './undo';

const region: RegionState = {
  inner: { x: 10, y: 20, w: 30, h: 20 },
  outer: { x: 0, y: 0, w: 50, h: 40 },
};
const view: MaskView = { reset: () => undefined, update: () => undefined };

interface Rig {
  ctl: MaskController;
  sync: MaskSync;
  sent: MaskBuffer[];
  statuses: MaskStatus[];
  readOnly: { value: boolean };
  fail: { next: ApiError | null };
}

function rig(): Rig {
  const sent: MaskBuffer[] = [];
  const statuses: MaskStatus[] = [];
  const readOnly = { value: false };
  const fail: { next: ApiError | null } = { next: null };
  let n = 0;
  const handoff: MaskHandoff = {
    submit: async (_pid, m) => {
      if (fail.next) {
        const e = fail.next;
        fail.next = null;
        throw e;
      }
      sent.push({ rect: { ...m.rect }, data: m.data.slice() });
      return { mask_id: `M${++n}`, rect: m.rect } satisfies MaskRef;
    },
  };
  let sync: MaskSync | null = null;
  const ctl = new MaskController({
    undo: new UndoStack(),
    view,
    setRegion: () => undefined,
    onChange: () => sync?.changed(),
  });
  sync = new MaskSync({
    projectId: 'P1',
    handoff,
    buffer: () => ctl.buffer,
    version: () => ctl.version,
    stroking: () => ctl.stroking,
    readOnly: () => readOnly.value,
    onStatus: (s) => statuses.push(s),
  });
  return { ctl, sync, sent, statuses, readOnly, fail };
}

/** A stroke across the region (image px). */
function stroke(ctl: MaskController, x0: number, x1: number) {
  ctl.beginStroke({ x: x0, y: 30 }, 'paint', 6, 1);
  ctl.moveStroke({ x: x1, y: 30 });
  ctl.endStroke();
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('mask hand-off', () => {
  test('FR-EDT-28 submits ≥ 2 s after the last stroke end, once, with the live bytes', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    stroke(r.ctl, 12, 20);
    expect(r.sync.current).toEqual({ kind: 'unsaved', reason: 'pending' });
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS - 1);
    stroke(r.ctl, 20, 30); // A new stroke restarts the debounce.
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS - 1);
    expect(r.sent).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(r.sent).toHaveLength(1);
    expect(r.sent[0]!.rect).toEqual(region.inner);
    expect(r.sent[0]!.data).toEqual(r.ctl.buffer!.data);
    expect(r.sync.current).toEqual({ kind: 'saved' });
    expect(r.statuses.map((s) => s.kind)).toContain('saving');
  });

  test('FR-EDT-28 never submits mid-stroke: a held stroke waits for its end', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    r.ctl.beginStroke({ x: 15, y: 30 }, 'paint', 6, 1);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 3);
    expect(r.sent).toHaveLength(0);
    r.ctl.endStroke();
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(1);
  });

  test('FR-EDT-28 is skipped when the mask hasn’t changed since the last successful submit', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    // An empty new mask isn't submitted.
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 2);
    expect(r.sent).toHaveLength(0);
    r.ctl.fillAll(255);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(1);
    // Clear then fill again: the same bytes as stored → no second submit.
    r.ctl.fillAll(0);
    r.ctl.fillAll(255);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(1);
    expect(r.sync.current).toEqual({ kind: 'saved' });
    expect(await r.sync.flush()).toEqual({ mask_id: 'M1', rect: region.inner });
    expect(r.sent).toHaveLength(1);
  });

  test('FR-EDT-28 R7 a job submit flushes at once and gets the new ref', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    stroke(r.ctl, 12, 30);
    const ref = await r.sync.flush();
    expect(ref?.mask_id).toBe('M1');
    expect(r.sent).toHaveLength(1);
    // The pending debounce was cancelled by the flush.
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 2);
    expect(r.sent).toHaveLength(1);
    // No region → nothing to flush.
    r.ctl.changeRegion(null);
    expect(await r.sync.flush()).toBeNull();
    expect(r.sync.current).toEqual({ kind: 'none' });
  });

  test('FR-EDT-29 read-only: painting works, nothing is submitted, the status says so', async () => {
    const r = rig();
    r.readOnly.value = true;
    r.ctl.changeRegion(region);
    stroke(r.ctl, 12, 30);
    expect(r.ctl.buffer!.data.some((v) => v > 0)).toBe(true);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 3);
    expect(await r.sync.flush()).toBeNull();
    expect(r.sent).toHaveLength(0);
    expect(r.sync.current).toEqual({ kind: 'unsaved', reason: 'read_only' });
    expect(maskStatusText(r.sync.current)).toBe('Read-only: mask not saved');
  });

  test('FR-EDT-28 EDT §11 a locked write keeps the mask, shows unsaved and retries', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    stroke(r.ctl, 12, 30);
    r.fail.next = new ApiError({ code: 'locked', message: 'the file is locked' });
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(0);
    expect(r.sync.current).toMatchObject({ kind: 'unsaved', reason: 'locked' });
    expect(maskStatusText(r.sync.current)).toMatch(/locked/);
    // The next commit (lock released) retries.
    r.sync.retry();
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(1);
    expect(r.sync.current).toEqual({ kind: 'saved' });
  });

  test('FR-EDT-28 a conflict on a job-submit flush rejects with the core code', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    stroke(r.ctl, 12, 30);
    r.fail.next = new ApiError({ code: 'conflict', message: 'sync conflict' });
    await expect(r.sync.flush()).rejects.toMatchObject({ code: 'conflict' });
    expect(r.sync.current).toMatchObject({ kind: 'unsaved', reason: 'conflict' });
    // The next trigger (a stroke) submits again.
    stroke(r.ctl, 30, 35);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(1);
  });

  test('FR-EDT-28 a change while saving is submitted after the debounce', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    r.ctl.fillAll(255);
    const p = r.sync.flush();
    r.ctl.fillAll(0); // changed while the submit is in flight
    await p;
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent.map((m) => m.data[0])).toEqual([255, 0]);
  });

  test('FR-EDT-19 a reopened mask is not resubmitted; a moved region is', async () => {
    const r = rig();
    const stored = emptyMask(region.inner);
    stored.data.fill(9);
    r.sync.markLoaded(stored, 'M0');
    r.ctl.load(region, stored);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 2);
    expect(r.sent).toHaveLength(0);
    expect(r.sync.current).toEqual({ kind: 'saved' });
    expect(await r.sync.flush()).toEqual({ mask_id: 'M0', rect: region.inner });

    const s = rig();
    s.sync.markLoaded(stored, 'M0');
    s.ctl.load({ ...region, inner: { ...region.inner, x: 20 } }, stored);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(s.sent).toHaveLength(1);
    expect(s.sent[0]!.rect.x).toBe(20);
  });
});

describe('BL-31 Keep other reload', () => {
  test('bl_31 a hold cancels the pending retry until the kept mask is loaded', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    stroke(r.ctl, 12, 30);
    r.fail.next = new ApiError({ code: 'conflict', message: 'sync conflict' });
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    r.sync.retry(); // the Keep other commit
    r.sync.hold(); // Keep other: reload the kept mask
    r.sync.retry(); // a later commit while the reload is still reading
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 3);
    expect(r.sent).toHaveLength(0);
    expect(r.sync.pending).toBe(false);
    const kept = emptyMask(region.inner);
    kept.data.fill(7);
    r.sync.markLoaded(kept, 'M-other');
    r.ctl.load(region, kept);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 2);
    expect(r.sent).toHaveLength(0);
    expect(r.sync.current).toEqual({ kind: 'saved' });
    expect(await r.sync.flush()).toEqual({ mask_id: 'M-other', rect: region.inner });
  });

  test('bl_31 a submit in flight when the hold starts is forgotten; a flush waits for the load', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    r.ctl.fillAll(255);
    void r.sync.flush(); // in flight
    r.sync.hold();
    await vi.advanceTimersByTimeAsync(0);
    const flushed = r.sync.flush();
    const kept = emptyMask(region.inner);
    kept.data.fill(7);
    r.sync.markLoaded(kept, 'M-other');
    r.ctl.load(region, kept);
    expect(await flushed).toEqual({ mask_id: 'M-other', rect: region.inner });
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 2);
    expect(r.sent).toHaveLength(1); // only the one already in flight
    expect(r.sync.current).toEqual({ kind: 'saved' });
  });

  test('bl_31 resume ends a hold without a loaded mask and submits a changed one again', async () => {
    const r = rig();
    r.ctl.changeRegion(region);
    r.sync.hold();
    stroke(r.ctl, 12, 30);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS * 2);
    expect(r.sent).toHaveLength(0);
    r.sync.resume();
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(1);
  });
});

describe('IPC body', () => {
  test('FR-EDT-18 the mask bytes go out as base64 and come back byte-identical', async () => {
    const all = Uint8Array.from({ length: 70_001 }, (_, i) => (i * 31) % 256);
    expect(fromBase64(toBase64(all))).toEqual(all);
    expect(toBase64(Uint8Array.of(104, 101, 108, 108, 111))).toBe('aGVsbG8=');

    vi.useRealTimers();
    const core = installTestCore();
    let stored: { rect: unknown; data_b64: string } | null = null;
    core.useHandlers({
      edt_mask_submit: (input) => {
        stored = { rect: input.rect, data_b64: String(input.data_b64) };
        return { mask_id: 'M9', rect: input.rect };
      },
      edt_mask_read: () => ({ mask_id: 'M9', ...stored! }),
    });
    const rect = { x: 1, y: 2, w: 7, h: 3 };
    const data = Uint8Array.from({ length: 21 }, (_, i) => i * 12);
    const ref = await coreHandoff.submit('P1', { rect, data });
    expect(ref).toEqual({ mask_id: 'M9', rect });
    expect(core.calls[0]!.input).toEqual({ project_id: 'P1', rect, data_b64: toBase64(data) });
    const { api } = await import('../../api');
    const back = await api.mask.read('P1', 'M9');
    expect(back.rect).toEqual(rect);
    expect(back.bytes).toEqual(data);
  });
});

describe('validated region from RGN', () => {
  test('BL-14 same inner: only the outline changes (no undo step); a new inner is a region change', () => {
    const undo = new UndoStack();
    const set: (RegionState | null)[] = [];
    const ctl = new MaskController({ undo, view, setRegion: (r) => set.push(r) });
    ctl.changeRegion(region);
    ctl.fillAll(255);
    const depth = undo.snapshot().depth;
    const wider = { inner: region.inner, outer: { x: -5, y: 0, w: 60, h: 40 } };
    ctl.applyValidated(wider);
    expect(ctl.currentRegion).toEqual(wider);
    expect(set.at(-1)).toEqual(wider);
    expect(undo.snapshot().depth).toBe(depth);
    expect(ctl.buffer!.data.every((v) => v === 255)).toBe(true);
    ctl.applyValidated(wider); // unchanged → nothing
    expect(set.at(-1)).toEqual(wider);

    const moved = { inner: { ...region.inner, x: 25 }, outer: wider.outer };
    ctl.applyValidated(moved);
    expect(ctl.currentRegion!.inner.x).toBe(25);
    expect(undo.snapshot().depth).toBe(depth + 1);
    // FR-EDT-20: the overlap (x 25..40 of the old 10..40) is kept, the rest is 0.
    expect(ctl.buffer!.data[0]).toBe(255);
    expect(ctl.buffer!.data[region.inner.w - 1]).toBe(0);
  });
});

describe('cost per dab', () => {
  test('R55 pending (the close guard holds only then): unsaved edits or a submit in flight', async () => {
    const r = rig();
    expect(r.sync.pending).toBe(false); // no region
    r.ctl.load(region, null);
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(r.sent).toHaveLength(0);
    expect(r.sync.pending).toBe(false); // an empty new mask isn't worth a file
    stroke(r.ctl, 12, 30);
    expect(r.sync.pending).toBe(true);
    await r.sync.flush();
    expect(r.sent).toHaveLength(1);
    expect(r.sync.pending).toBe(false);
    stroke(r.ctl, 12, 30);
    r.readOnly.value = true;
    expect(r.sync.pending).toBe(false); // read-only never submits (FR-EDT-29)
  });

  test('FR-EDT-28 §12 a dab never scans the buffer; the debounce checks it once', async () => {
    const r = rig();
    // A new, never-saved 4096² mask: the "is it still empty?" check is a full scan.
    r.ctl.changeRegion({ inner: { x: 0, y: 0, w: 4096, h: 4096 }, outer: region.outer });
    const every = vi.spyOn(Uint8Array.prototype, 'every');
    r.ctl.beginStroke({ x: 100, y: 100 }, 'paint', 8, 1);
    for (let i = 0; i < 50; i++) r.ctl.moveStroke({ x: 100 + i * 4, y: 100 });
    r.ctl.endStroke();
    expect(every).not.toHaveBeenCalled();
    expect(r.sync.current).toEqual({ kind: 'unsaved', reason: 'pending' });
    await vi.advanceTimersByTimeAsync(MASK_SUBMIT_DEBOUNCE_MS);
    expect(every).toHaveBeenCalledTimes(1);
    expect(r.sent).toHaveLength(1);
  });
});
