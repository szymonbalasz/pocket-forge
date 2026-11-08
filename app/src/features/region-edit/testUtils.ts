// Vitest helpers for region-edit (test-only; never imported by app code): the capabilities
// fixture, jsdom polyfills Radix needs, and a mocked core (fake Tauri + the rgn mock) with an
// open project.
import allTypes from '../../../../fixtures/rgn/capabilities-all-types.json';
import type { Capabilities } from '../../api';
import { installRgnMock, type RgnMockSeed } from '../../api/mocks/rgn';
import { baseSnapshot, installFakeTauri, type FakeTauri } from '../../api/mocks/tauri';

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

/** Installs the mocked core; `project_open` returns the base snapshot + the mock's fields. */
export function mockCore(patch: Partial<RgnMockSeed> = {}): FakeTauri {
  const fake = installFakeTauri(baseSnapshot());
  const w = window as unknown as {
    __pfMockHandlers: Record<string, (i: unknown) => unknown>;
    __pfMockSnapshot: Record<string, unknown>;
  };
  installRgnMock(seedOf(patch));
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
  setCancel(c: 'canceled' | 'not_cancellable'): void;
  failNext(cmd: string, code: string, message?: string, field?: string): void;
  state(): { spend: number; estimates: number; cap: number | null; threshold: number | null };
}

export const mockControls = (): RgnMockControls =>
  (window as unknown as { __pfRgnMock: RgnMockControls }).__pfRgnMock;
