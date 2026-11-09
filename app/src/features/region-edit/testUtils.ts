// Vitest helpers for region-edit (test-only; never imported by app code): the capabilities
// fixture, jsdom polyfills Radix needs, and a mocked core (fake Tauri + the rgn mock) with an
// open project.
import allTypes from '../../../../fixtures/rgn/capabilities-all-types.json';
import type { Capabilities } from '../../api';
import { installPrmMock, type PrmMockSeed } from '../../api/mocks/prm';
import { installRgnMock, type RgnMockSeed } from '../../api/mocks/rgn';
import { baseSnapshot, installFakeTauri, type FakeTauri } from '../../api/mocks/tauri';
import { resetPromptHelpers } from '../prompt-helper';

/** `fixtures/rgn/capabilities-all-types.json` (RGN §13). */
export const ALL_TYPES = allTypes as Capabilities;

/** A second model with a small, different capability set (FR-RGN-07/08). */
export const SIMPLE: Capabilities = {
  ...ALL_TYPES,
  model_id: 'fake-simple',
  controls: ALL_TYPES.controls.filter((c) =>
    ['image', 'mask', 'text', 'num_samples', 'seed'].includes(c.name),
  ),
  rules: [],
  slots: [],
  warnings: [],
};

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

export function seedOf(patch: Partial<RgnMockSeed> = {}): RgnMockSeed {
  return {
    image: { w: 2048, h: 2560 },
    models: [
      { id: 'fake-all-types', name: 'All types' },
      { id: 'fake-simple', name: 'Simple' },
    ],
    capabilities: { 'fake-all-types': ALL_TYPES, 'fake-simple': SIMPLE },
    spend: { cap: 100, threshold: 10 },
    ...patch,
  };
}

/** Installs the mocked core; `project_open` returns the base snapshot + the mock's fields. The
 *  PRM mock backs the prompt block (prompt-helper §7); `prm` seeds its helper list. */
export function mockCore(patch: Partial<RgnMockSeed> = {}, prm: PrmMockSeed = {}): FakeTauri {
  resetPromptHelpers();
  const fake = installFakeTauri(baseSnapshot());
  const w = window as unknown as {
    __pfMockHandlers: Record<string, (i: unknown) => unknown>;
    __pfMockSnapshot: Record<string, unknown>;
  };
  installRgnMock(seedOf(patch));
  installPrmMock(prm);
  w.__pfMockHandlers.project_open = () => ({ ...baseSnapshot(), ...w.__pfMockSnapshot });
  w.__pfMockHandlers.provider_status = () => ({
    mode: 'fake',
    configured: true,
    connectivity: 'ok',
    store_unavailable: false,
  });
  return fake;
}

export interface RgnMockControls {
  run(progress?: number, extra?: Record<string, unknown>): void;
  finish(count?: number, invalid?: number[]): void;
  fail(code?: string, hint?: string): void;
  fetchError(code?: string, hint?: string): void;
  spool(on?: boolean): void;
  lose(handleKept?: boolean, code?: string, hint?: string): void;
  setCancel(c: 'canceled' | 'not_cancellable'): void;
  failNext(cmd: string, code: string, message?: string, field?: string): void;
  state(): { spend: number; estimates: number; cap: number | null; threshold: number | null };
}

export const mockControls = (): RgnMockControls =>
  (window as unknown as { __pfRgnMock: RgnMockControls }).__pfRgnMock;
