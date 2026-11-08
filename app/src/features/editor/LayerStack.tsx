// BL-22 (OQ-PRJ-11 A, #41): draws a given layer stack read-only at fit-to-pane size, for other
// features (the conflict view). The original and the patch layers are the core's files as baked
// (feather and colour match are already in the PNG, ADR 0004); each patch is placed with the
// manifest affine (ADR 0003): centre at (cx, cy), scaled and rotated about its centre. If a file
// can't be drawn (not synced yet, unreadable), only the caption shows, so the user never sees
// a partial stack as if it were the version.
import { useEffect, useState, type ReactNode } from 'react';

import { api, type Layer } from '../../api';
import './editor.css';

/** The original is drawn from a cached copy at most this long (the pane is ~360 px wide). */
export const STACK_ORIGINAL_MAX_SIDE = 1024;

interface Loaded {
  url: string;
  w: number;
  h: number;
}

type LoadState = Loaded | 'failed';

export interface LayerStackProps {
  projectId: string | null;
  /** The stack in draw order, original first (a manifest's `layers`). */
  layers: readonly Layer[];
  /** Project paths known not to be here yet: their layers aren't requested. */
  missingFiles?: readonly string[];
  /** `data-testid` of the figure; the images add `-original` and `-layer`. */
  testId: string;
  /** Text under the drawing, and the whole content when it can't be drawn. */
  caption: ReactNode;
}

const pct = (v: number) => `${v * 100}%`;
const NONE: readonly string[] = [];

export function LayerStack({
  projectId,
  layers,
  missingFiles = NONE,
  testId,
  caption,
}: LayerStackProps) {
  const [loaded, setLoaded] = useState<Record<string, LoadState>>({});

  const original = layers.find((l) => l.kind === 'original');
  const patches = layers.filter((l) => l.kind === 'patch' && l.visible);
  const missing = new Set(missingFiles);
  const isMissing = (l: Layer) => l.file !== null && missing.has(l.file);

  useEffect(() => {
    if (!projectId) return;
    let live = true;
    const miss = new Set(missingFiles);
    for (const l of layers) {
      const drawn = l.kind === 'original' || l.visible;
      if (!drawn || (l.file !== null && miss.has(l.file))) continue;
      const open =
        l.kind === 'original'
          ? api.editor.imageOpen(projectId, l.id, STACK_ORIGINAL_MAX_SIDE)
          : api.editor.imageOpen(projectId, l.id);
      open.then(
        (img) => {
          if (live)
            setLoaded((s) => ({ ...s, [l.id]: { url: img.url, w: img.width, h: img.height } }));
        },
        () => {
          if (live) setLoaded((s) => ({ ...s, [l.id]: 'failed' }));
        },
      );
    }
    return () => {
      live = false;
    };
  }, [projectId, layers, missingFiles]);

  const fail = (id: string) => setLoaded((s) => ({ ...s, [id]: 'failed' }));
  const needed = original ? [original, ...patches] : [];
  const failed =
    !projectId || !original || needed.some((l) => isMissing(l) || loaded[l.id] === 'failed');
  const state = failed ? 'fallback' : needed.every((l) => loaded[l.id]) ? 'drawn' : 'loading';

  if (failed || !original) {
    return (
      <figure className="edt-stack" data-testid={testId} data-state={state}>
        <figcaption>{caption}</figcaption>
      </figure>
    );
  }

  // The original layer sits at the image centre, scale 1 (manifest Layer::original): its
  // extent is the image's.
  const W = original.cx * 2;
  const H = original.cy * 2;
  const base = loaded[original.id];
  return (
    <figure className="edt-stack" data-testid={testId} data-state={state}>
      <div
        className="edt-stack-box"
        style={{
          aspectRatio: `${W} / ${H}`,
          width: `min(100%, calc(var(--edt-stack-max-h) * ${W / H}))`,
        }}
      >
        {base && base !== 'failed' && (
          <img
            className="edt-stack-original"
            data-testid={`${testId}-original`}
            data-record-id={original.id}
            src={base.url}
            alt=""
            onError={() => fail(original.id)}
          />
        )}
        {patches.map((l) => {
          const img = loaded[l.id];
          if (!img || img === 'failed') return null;
          return (
            <img
              key={l.id}
              className="edt-stack-layer"
              data-testid={`${testId}-layer`}
              data-record-id={l.id}
              src={img.url}
              alt=""
              onError={() => fail(l.id)}
              style={{
                left: pct(l.cx / W),
                top: pct(l.cy / H),
                width: pct((img.w * l.scale) / W),
                height: pct((img.h * l.scale) / H),
                opacity: l.opacity,
                transform: `translate(-50%, -50%) rotate(${l.rotation}deg)`,
              }}
            />
          );
        })}
      </div>
      <figcaption>{caption}</figcaption>
    </figure>
  );
}
