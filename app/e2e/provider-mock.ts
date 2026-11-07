// PRV e2e helpers on top of mock-core: provider command handlers per test
// (window.__pfMockHandlers), seeded with a mode, a stored key and a test-connection result.
import type { Page } from '@playwright/test';

import { mockCore } from './mock-core';

export interface ProviderSeed {
  mode: 'fake' | 'scenario';
  stored: string | null;
  connectivity: 'unknown' | 'ok' | 'offline' | 'auth_error';
  testResult: 'ok' | 'auth' | 'offline';
  testDelayMs: number;
}

export async function mockProvider(page: Page, seed: ProviderSeed): Promise<void> {
  await mockCore(page);
  await page.addInitScript((seed: ProviderSeed) => {
    type Rec = Record<string, unknown>;
    const extra = ((window as unknown as Rec).__pfMockHandlers ??= {}) as Record<
      string,
      (input: Rec) => unknown
    >;
    const s = { ...seed, team: null as string | null, project: null as string | null };
    const status = () => ({
      mode: s.mode,
      configured: s.mode === 'fake' || s.stored !== null,
      ...(s.stored ? { key_suffix: s.stored.slice(-4) } : {}),
      connectivity: s.connectivity,
      store_unavailable: false,
    });
    const providerErr = (code: string) =>
      Promise.reject({ code: 'provider', message: code, detail: { provider: { code } } });
    const handlers: Record<string, (input: Rec) => unknown> = {
      provider_status: status,
      provider_balance: () =>
        s.connectivity === 'ok' || s.mode === 'fake'
          ? {
              kind: 'known',
              remaining: 1234,
              unit: s.mode === 'fake' ? 'fake' : 'CU',
              period_end: null,
            }
          : { kind: 'unknown' },
      provider_credentials_set: (input) => {
        s.stored = String(input.key);
        s.connectivity = 'unknown';
        return {};
      },
      provider_credentials_clear: () => {
        s.stored = null;
        s.connectivity = 'unknown';
        return {};
      },
      provider_settings_get: () => ({ team_id: s.team, project_id: s.project }),
      provider_settings_set: (input) => {
        s.team = (input.team_id as string | null) ?? null;
        s.project = (input.project_id as string | null) ?? null;
        return { team_id: s.team, project_id: s.project };
      },
      provider_test_connection: async () => {
        await new Promise((r) => setTimeout(r, s.testDelayMs));
        if (s.stored === null) return Promise.reject({ code: 'not_configured', message: 'x' });
        if (s.testResult === 'ok') {
          s.connectivity = 'ok';
          return {};
        }
        s.connectivity = s.testResult === 'auth' ? 'auth_error' : 'offline';
        return providerErr(s.testResult);
      },
    };
    Object.assign(extra, handlers);
  }, seed);
  await page.goto('/');
}
