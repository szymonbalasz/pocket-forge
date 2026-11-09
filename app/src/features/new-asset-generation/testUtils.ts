// Vitest helpers for the New image tab (test-only; never imported by app code): the two fake
// generate models as new-asset-generation §10 describes them, and a mocked core (fake Tauri + the
// RGN, GEN and REF mocks) with an open project of a given kind.
import generateCaps from '../../../../fixtures/gen/capabilities-generate.json';
import allTypes from '../../../../fixtures/rgn/capabilities-all-types.json';
import type { Capabilities, FolderFile, ProjectKind } from '../../api';
import { installGenMock, type GenMockSeed } from '../../api/mocks/gen';
import { refMock, sampleRefSeed } from '../../api/mocks/ref';
import { installRgnMock } from '../../api/mocks/rgn';
import { baseSnapshot, installFakeTauri, type FakeTauri } from '../../api/mocks/tauri';

/** `fixtures/gen/capabilities-generate.json` (§13): the fake generate models as §10 describes
 *  them. `fake-gen-wh`: wh 64–2048 step 16, count 1–4, a `background` enum with transparent, one
 *  cost-affecting slot ≤ 4 that also takes the sketch. `fake-gen-aspect`: ratios 1:1, 3:4, 4:3,
 *  2:3 (plus the keyword `auto`), no count, a style slot ≤ 2, no sketch. */
export const GEN_WH = generateCaps['fake-gen-wh'] as Capabilities;
export const GEN_ASPECT = generateCaps['fake-gen-aspect'] as Capabilities;

export const GEN_MODELS = [
  { id: 'fake-gen-wh', name: 'Fake gen (wh)' },
  { id: 'fake-gen-aspect', name: 'Fake gen (aspect)' },
];

export function polyfillRadix(): void {
  const g = globalThis as unknown as Record<string, unknown>;
  g.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => undefined;
  proto.releasePointerCapture ??= () => undefined;
  proto.scrollIntoView ??= () => undefined;
}

export interface GenSetup {
  kind?: ProjectKind;
  /** A project made by Generate new: no image yet (FR-GEN-02). */
  noImage?: boolean;
  readOnly?: boolean;
  /** `structure/` images. */
  sketches?: FolderFile[];
  /** The project's `references/` images. */
  projectRefs?: string[];
  gen?: Partial<GenMockSeed>;
  /** `draft.generate` saved in the project. */
  draft?: Record<string, unknown> | null;
}

export const PROJECT_DIR = 'C:/pf-test/library/Maps/map';

/** Installs the mocked core; `project_open` returns the snapshot of the asked project. */
export function mockGenCore(opts: GenSetup = {}): FakeTauri & { structure: FolderFile[] } {
  const snap = {
    ...baseSnapshot(),
    kind: opts.kind ?? 'map',
    ...(opts.noImage ? { image_path: null, width: 0, height: 0 } : {}),
  };
  const fake = installFakeTauri(snap);
  const w = window as unknown as {
    __pfMockHandlers: Record<string, (i: Record<string, unknown>) => unknown>;
    __pfMockSnapshot: Record<string, unknown>;
  };
  installRgnMock({
    image: { w: 2048, h: 2560 },
    models: [{ id: 'fake-all-types', name: 'All types' }],
    capabilities: { 'fake-all-types': allTypes as Capabilities },
    spend: { cap: 100, threshold: 10 },
    readOnly: opts.readOnly,
    kind: opts.kind,
  });
  installGenMock({
    models: GEN_MODELS,
    capabilities: { 'fake-gen-wh': GEN_WH, 'fake-gen-aspect': GEN_ASPECT },
    costPerImage: 1,
    ...opts.gen,
  });
  const refs = refMock(
    {
      ...sampleRefSeed(),
      slots: {
        'fake-all-types': [],
        'fake-gen-wh': [{ name: 'reference_images', role: null, min: 0, max: 4 }],
        'fake-gen-aspect': [{ name: 'style_images', role: 'style', min: 0, max: 2 }],
      },
      structureSlots: { 'fake-gen-wh': 'reference_images' },
      projectRefs: (opts.projectRefs ?? []).map((file) => ({ file })),
      readOnly: opts.readOnly,
    },
    { emit: fake.emit },
  );
  const structure: FolderFile[] = [...(opts.sketches ?? [])];
  if (opts.draft !== undefined)
    w.__pfMockSnapshot.draft = { ...(w.__pfMockSnapshot.draft as object), generate: opts.draft };
  Object.assign(w.__pfMockHandlers, refs.handlers, {
    project_open: () => ({ ...snap, ...w.__pfMockSnapshot }),
    provider_status: () => ({
      mode: 'fake',
      configured: true,
      connectivity: 'ok',
      store_unavailable: false,
    }),
    project_folder_list: (input: Record<string, unknown>) =>
      input.folder === 'structure' ? [...structure] : [],
    project_folder_add: (input: Record<string, unknown>) => {
      for (const p of input.paths as string[])
        structure.push({
          name: p.split(/[\\/]/).pop()!,
          bytes: 2048,
          mtime: '2026-11-09T10:00:00.000Z',
          image: true,
        });
      return [...structure];
    },
  });
  return Object.assign(fake, { structure });
}

export const sketch = (name: string, state?: FolderFile['state']): FolderFile => ({
  name,
  bytes: 4096,
  mtime: '2026-11-09T10:00:00.000Z',
  image: true,
  ...(state ? { state } : {}),
});

export interface Controls {
  run(progress?: number): void;
  finish(count?: number): void;
}
export const rgnControls = (): Controls =>
  (window as unknown as { __pfRgnMock: Controls }).__pfRgnMock;
export const genControls = () =>
  (window as unknown as { __pfGenMock: { changeSketch(file: string): void } }).__pfGenMock;
