// Region tool + mask tools + undo, wired to the canvas (FR-EDT-14..17, 19..23, 27).
// The canvas forwards pointer events here when the active tool is region, brush or eraser.
import type Konva from 'konva';
import { useCallback, useEffect, useRef, useState } from 'react';

import { api, onEvent, type ProjectSnapshot } from '../../api';
import { projectStore } from '../../store/project';
import { bindActions } from './actions';
import { bindRegionSink, setActiveMaskSync } from './bridge';
import { maskHash, type MaskBuffer } from './mask';
import { MaskController } from './MaskController';
import { coreHandoff, MaskSync } from './maskHandoff';
import { konvaMaskView } from './maskView';
import { applyDrag, insideRect, type Handle, type RegionDrag } from './region';
import { fitRegion, RegionError } from './regionFit';
import { RegionOverlay } from './RegionOverlay';
import { pushUndo, revisionUndoEntry, undoStack } from './session';
import { dispatchShortcut, type ShortcutMap } from './shortcuts';
import { editorStore, useEditor, type Rect } from './store';
import { screenToImage, type Pt } from './view';

export function useMaskTools({
  project,
  maskLayer,
  container,
}: {
  project: ProjectSnapshot;
  /** The mounted mask layer (state, so a remounted Stage rebinds the view). */
  maskLayer: Konva.Layer | null;
  container: React.RefObject<HTMLDivElement | null>;
}) {
  const { view, tool, brushSize, hardness, region, image, originalMissing } = useEditor((s) => s);
  const ctl = useRef<MaskController | null>(null);
  const drag = useRef<{ d: RegionDrag; pointerId: number } | null>(null);
  const stroking = useRef<number | null>(null);
  const [preview, setPreview] = useState<Rect | null>(null);
  const [cursor, setCursor] = useState<Pt | null>(null);
  const [regionError, setRegionError] = useState<{ code: string; message: string } | null>(null);
  const [maskWarning, setMaskWarning] = useState<string | null>(null);
  // FR-EDT-29: read-only follows the project (a relink clears it without a remount).
  const readOnly = useRef(project.read_only !== null);
  readOnly.current = project.read_only !== null;
  const sync = useRef<MaskSync | null>(null);

  // One controller per project; its view follows the mask layer instance. The Stage can remount
  // (the viewport drops to 0 when the store resets on open), so a new layer gets a new view
  // that redraws the current mask, and the old view is destroyed (D1).
  useEffect(() => {
    if (!maskLayer) return;
    const mv = konvaMaskView(maskLayer);
    if (ctl.current) ctl.current.setView(mv);
    else {
      const c = new MaskController({
        undo: undoStack,
        view: mv,
        setRegion: (r) => editorStore.set({ region: r }),
        onChange: () => sync.current?.changed(),
      });
      ctl.current = c;
      // FR-EDT-28: the hand-off for this project (debounce, job submit, close).
      sync.current = new MaskSync({
        projectId: project.project_id,
        handoff: coreHandoff,
        buffer: () => c.buffer,
        version: () => c.version,
        stroking: () => c.stroking,
        readOnly: () => readOnly.current,
        onStatus: (maskStatus) => editorStore.set({ maskStatus }),
      });
      setActiveMaskSync(sync.current);
    }
    return () => mv.destroy();
  }, [maskLayer, project.project_id]);

  // FR-EDT-28: submit before the project closes, on PRJ's close guard (R55); a failed submit still
  // closes (the status said unsaved). Retry a failed submit after the next commit (a resolved
  // conflict, a released lock); a read-only change refreshes the status.
  useEffect(() => {
    bindRegionSink((r) => ctl.current?.applyValidated(r));
    const off = projectStore.addCloseGuard((proceed) => {
      const s = sync.current;
      if (!s?.pending) return true;
      void s
        .flush()
        .catch(() => undefined)
        .then(proceed);
      return false;
    });
    const sub = onEvent('project:committed', () => sync.current?.retry());
    return () => {
      bindRegionSink(null);
      off();
      void sub.then(
        (u) => u(),
        () => undefined,
      );
    };
  }, []);
  useEffect(() => {
    sync.current?.changed();
  }, [project.read_only]);

  // FR-EDT-19: reopen loads the draft's mask once per controller, when the image size is known;
  // a different draft region keeps the overlap.
  const draftLoadedFor = useRef<MaskController | null>(null);
  // Walk 2 N1, Keep other: load the kept draft's mask as at reopen, so a retried submit can't
  // write this machine's mask over it (RGN sends the kept region, BL-26). Keep this keeps the
  // mask and its retry. BL-31: a hard hold on the hand-off until the reload lands, so the 2 s
  // retry (or a submit in flight) can't race it.
  useEffect(
    () =>
      projectStore.onResolved((p, keep) => {
        if (keep !== 'other' || p.project_id !== project.project_id) return;
        draftLoadedFor.current = null;
        sync.current?.hold();
      }),
    [project.project_id],
  );
  useEffect(() => {
    const c = ctl.current;
    // BL-10: with the original missing, the draft loads after the relink.
    if (!c || !image || originalMissing || draftLoadedFor.current === c) return;
    draftLoadedFor.current = c;
    const draft = project.draft as {
      mask_id?: string | null;
      region?: { inner?: Rect; margin?: number } | null;
    } | null;
    const inner = draft?.region?.inner;
    const restored = () => {
      if (ctl.current === c) editorStore.set({ regionFor: project.project_id });
    };
    if (!inner) {
      sync.current?.resume();
      restored();
    } else {
      void (async () => {
        try {
          const r = await fitRegion(project.project_id, inner, draft?.region?.margin);
          let m: MaskBuffer | null = null;
          if (draft?.mask_id) {
            try {
              const read = await api.mask.read(project.project_id, draft.mask_id);
              m = { rect: read.rect, data: read.bytes };
              if (m.data.length !== m.rect.w * m.rect.h) throw new Error('size mismatch');
            } catch (e) {
              // A corrupt or unreadable mask: start empty with a warning (EDT §11).
              setMaskWarning(`The saved mask could not be loaded (${(e as Error).message}).`);
              m = null;
            }
          }
          if (ctl.current !== c) return;
          // The store holds `m`: no submit unless the region changed it (FR-EDT-19).
          sync.current?.markLoaded(m, m ? (draft?.mask_id ?? null) : null);
          c.load(r, m);
        } catch {
          // The saved region no longer fits: start without one.
          if (ctl.current === c) sync.current?.resume();
        }
        restored();
      })();
    }
    return undefined;
  }, [maskLayer, project.project_id, project.draft, image, originalMissing]);

  useEffect(
    () => () => {
      ctl.current = null;
      sync.current?.dispose();
      sync.current = null;
      setActiveMaskSync(null);
      editorStore.set({ maskStatus: { kind: 'none' } });
      undoStack.clear(); // KQ-16: history is per session.
      // The region left behind is this session's; a reopen restores it again first.
      editorStore.set({ regionFor: null });
    },
    [],
  );

  // Undo summary → store (toolbar tooltip, FR-EDT-22) and action bindings.
  useEffect(() => {
    const off = undoStack.subscribe((s) =>
      editorStore.set({
        undo: { depth: s.depth, redo: s.redo, label: s.label, busy: s.busy, error: s.error },
      }),
    );
    const unbind = bindActions({
      fill: () => ctl.current?.fillAll(255),
      clear: () => ctl.current?.fillAll(0),
      undo: () => void undoStack.undo(),
      redo: () => void undoStack.redo(),
    });
    return () => {
      off();
      unbind();
    };
  }, []);

  // FR-EDT-21 / FR-EDT-27 shortcuts.
  useEffect(() => {
    const map: ShortcutMap = {
      'Ctrl+Z': () => void undoStack.undo(),
      'Ctrl+Shift+Z': () => void undoStack.redo(),
      'Ctrl+Y': () => void undoStack.redo(),
      Delete: () => {
        if (ctl.current?.currentRegion) ctl.current.changeRegion(null);
      },
    };
    const down = (e: KeyboardEvent) => void dispatchShortcut(e, map);
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, []);

  const imagePoint = useCallback(
    (e: React.PointerEvent): Pt => {
      const r = container.current!.getBoundingClientRect();
      return screenToImage(editorStore.get().view, { x: e.clientX - r.left, y: e.clientY - r.top });
    },
    [container],
  );

  /** Emits `{inner}` to RGN's fit and applies the validated region (FR-EDT-27, R18). */
  const commitRegion = useCallback(
    async (inner: Rect) => {
      const img = editorStore.get().image;
      if (!img) return;
      try {
        const r = await fitRegion(project.project_id, inner);
        setRegionError(null);
        ctl.current?.changeRegion(r);
      } catch (e) {
        if (e instanceof RegionError) setRegionError({ code: e.code, message: e.message });
        else setRegionError({ code: 'ui', message: String(e) });
      }
    },
    [project.project_id],
  );

  /** Returns true when the event was handled (the canvas then skips panning). */
  const onPointerDown = (e: React.PointerEvent): boolean => {
    if (e.button !== 0 || !image) return false;
    const p = imagePoint(e);
    if (tool === 'region') {
      const handle = (e.target as HTMLElement).dataset?.handle as Handle | undefined;
      const inner = region?.inner;
      const d: RegionDrag =
        handle && inner
          ? { kind: 'resize', handle, rect: inner }
          : inner && insideRect(inner, p)
            ? { kind: 'move', start: p, rect: inner }
            : { kind: 'new', start: p };
      drag.current = { d, pointerId: e.pointerId };
      setPreview(d.kind === 'new' ? null : applyDrag(d, p, image));
      return true;
    }
    if (tool === 'brush' || tool === 'eraser') {
      const c = ctl.current;
      const { brushSize: size, hardness: h } = editorStore.get();
      if (c?.beginStroke(p, tool === 'brush' ? 'paint' : 'erase', size, h)) {
        stroking.current = e.pointerId;
      }
      return true;
    }
    return false;
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const r = container.current?.getBoundingClientRect();
    if (r) setCursor({ x: e.clientX - r.left, y: e.clientY - r.top });
    if (!image) return;
    const g = drag.current;
    if (g && g.pointerId === e.pointerId) setPreview(applyDrag(g.d, imagePoint(e), image));
    if (stroking.current === e.pointerId) ctl.current?.moveStroke(imagePoint(e));
  };

  const onPointerUp = (e: React.PointerEvent) => {
    const g = drag.current;
    if (g && g.pointerId === e.pointerId && image) {
      drag.current = null;
      const rect = applyDrag(g.d, imagePoint(e), image);
      setPreview(null);
      // A click without a drag doesn't make a region.
      const moved = g.d.kind !== 'new' || rect.w > 1 || rect.h > 1;
      if (moved) void commitRegion(rect);
    }
    if (stroking.current === e.pointerId) {
      stroking.current = null;
      ctl.current?.endStroke();
    }
  };

  const overlay = (
    <RegionOverlay
      view={view}
      region={region}
      preview={preview}
      tool={tool}
      cursor={cursor}
      brushSize={brushSize}
      hardness={hardness}
      onLeave={() => setCursor(null)}
      regionError={regionError}
      maskWarning={maskWarning}
    />
  );

  const testExtras = () => ({
    region: () => ctl.current?.currentRegion ?? null,
    mask: () => {
      const m = ctl.current?.buffer;
      if (!m) return null;
      let sum = 0;
      let full = 0;
      for (const v of m.data) {
        sum += v;
        if (v === 255) full++;
      }
      return { rect: m.rect, hash: maskHash(m.data), sum, full, length: m.data.length };
    },
    /** Mask bytes of a sub-rect in mask px (keep it small). */
    maskBytes: (x: number, y: number, w: number, h: number) => {
      const m = ctl.current?.buffer;
      if (!m) return null;
      const out: number[] = [];
      for (let j = y; j < y + h; j++)
        for (let i = x; i < x + w; i++) out.push(m.data[j * m.rect.w + i] ?? -1);
      return out;
    },
    undoDepth: () => undoStack.snapshot(),
    maskStatus: () => sync.current?.current ?? null,
    /** FR-EDT-28: the job-submit trigger, as RGN calls it. */
    flushMask: () => sync.current?.flush() ?? null,
    maskVersion: () => ctl.current?.version ?? 0,
    /** FR-EDT-23: push an accept-style entry (undo/redo restore revisions). */
    pushRevisionEntry: (parent: string, later: string) =>
      pushUndo(revisionUndoEntry('accept', parent, later)),
  });

  return { onPointerDown, onPointerMove, onPointerUp, overlay, testExtras, controller: ctl };
}
