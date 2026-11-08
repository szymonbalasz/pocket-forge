// REF e2e helpers on top of mock-core: installs the mocked REF commands (src/api/mocks/ref.ts,
// the same mock Vitest uses) before the page loads; `window.__pfRefMock` is its state.
import type { Page } from '@playwright/test';

import { refMock, sampleRefSeed, type RefMockSeed } from '../src/api/mocks/ref';
import { LIBRARY } from './mock-core';

export function refSeed(patch: Partial<RefMockSeed> = {}): RefMockSeed {
  return { ...sampleRefSeed(LIBRARY), ...patch };
}

/** Registers the REF handlers. Call before `mockCore` or after: they are looked up per call. */
export async function mockRefs(page: Page, seed: RefMockSeed = refSeed()): Promise<void> {
  await page.addInitScript({
    content: `(() => {
      const w = window;
      const m = (${refMock.toString()})(${JSON.stringify(seed)}, {
        emit: (e, p) => w.__pfMock && w.__pfMock.emit(e, p),
      });
      w.__pfRefMock = m.state;
      Object.assign((w.__pfMockHandlers ??= {}), m.handlers);
    })();`,
  });
}
