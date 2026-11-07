// Provider settings and status chip (provider spec §7, §13 thin e2e), mocked core (KQ-33).
// The shared mock core answers the other commands; this file layers a provider mock on top by
// wrapping its invoke, so mock-core.ts stays untouched.
import { expect, test, type Page } from '@playwright/test';

import { calls, mockCore } from './mock-core';

interface ProviderSeed {
  mode: 'fake' | 'scenario';
  stored: string | null;
  connectivity: 'unknown' | 'ok' | 'offline' | 'auth_error';
  testResult: 'ok' | 'auth' | 'offline';
  testDelayMs: number;
}

async function mockProvider(page: Page, seed: ProviderSeed): Promise<void> {
  await mockCore(page);
  await page.addInitScript((seed: ProviderSeed) => {
    type Rec = Record<string, unknown>;
    const w = window as unknown as Rec & { __TAURI_INTERNALS__: Rec };
    const inner = w.__TAURI_INTERNALS__.invoke as (cmd: string, args: Rec) => Promise<unknown>;
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
    w.__TAURI_INTERNALS__.invoke = (cmd: string, args: Rec) => {
      const h = handlers[cmd];
      if (!h) return inner(cmd, args);
      ((w.__pfMock as Rec).calls as unknown[]).push({ cmd, args });
      try {
        return Promise.resolve(h((args?.input as Rec) ?? {}));
      } catch (e) {
        return Promise.reject(e);
      }
    };
  }, seed);
  await page.goto('/');
}

const base: ProviderSeed = {
  mode: 'scenario',
  stored: null,
  connectivity: 'unknown',
  testResult: 'auth',
  testDelayMs: 0,
};

test('FR-PRV-07 fake mode shows the fake badge with its balance', async ({ page }) => {
  await mockProvider(page, { ...base, mode: 'fake' });
  const chip = page.getByTestId('prv-header-chip');
  await expect(chip).toHaveAttribute('data-state', 'fake');
  await expect(chip).toHaveText('Fake provider · 1,234 fake');
});

test('FR-PRV-06 FR-PRV-07 save clears the fields, stores the key and shows its suffix', async ({
  page,
}) => {
  await mockProvider(page, base);
  const chip = page.getByTestId('prv-header-chip');
  await expect(chip).toHaveAttribute('data-state', 'not_configured');
  await chip.click();
  await page.getByTestId('prv-settings-key').fill('dummy-key-abcd');
  await page.getByTestId('prv-settings-secret').fill('dummy-secret');
  await page.getByTestId('prv-settings-team').fill('team_1');
  await page.getByTestId('prv-settings-save').click();
  await expect(page.getByTestId('prv-settings-result')).toHaveAttribute('data-state', 'ok');
  await expect(page.getByTestId('prv-settings-key')).toHaveValue('');
  await expect(page.getByTestId('prv-settings-secret')).toHaveValue('');
  await expect(chip).toHaveAttribute('data-state', 'configured');
  await expect(chip).toHaveText('Scenario · configured …abcd · unknown');
  const sent = (await calls(page)).filter((c) => c.cmd.startsWith('provider_'));
  expect(sent).toContainEqual({
    cmd: 'provider_credentials_set',
    args: { input: { key: 'dummy-key-abcd', secret: 'dummy-secret' } },
  });
  expect(sent).toContainEqual({
    cmd: 'provider_settings_set',
    args: { input: { team_id: 'team_1', project_id: null } },
  });

  await page.getByTestId('prv-settings-clear').click();
  await expect(chip).toHaveAttribute('data-state', 'not_configured');
  await expect(page.getByTestId('prv-settings-stored')).toHaveText('No key stored.');
});

test('FR-PRV-07 test connection shows a spinner, then auth error links to settings', async ({
  page,
}) => {
  await mockProvider(page, { ...base, stored: 'k-wxyz', testDelayMs: 300 });
  const chip = page.getByTestId('prv-header-chip');
  await chip.click();
  await page.getByTestId('prv-settings-test').click();
  await expect(page.getByTestId('prv-settings-spinner')).toBeVisible();
  await expect(page.getByTestId('prv-settings-result')).toHaveText(
    'Scenario rejected the key and secret.',
  );
  await expect(chip).toHaveAttribute('data-state', 'auth_error');
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('prv-settings-panel')).toBeHidden();
  await chip.click();
  await expect(page.getByTestId('prv-settings-panel')).toBeVisible();
});

test('FR-PRV-07 an unreachable provider shows offline', async ({ page }) => {
  await mockProvider(page, { ...base, stored: 'k-wxyz', testResult: 'offline' });
  const chip = page.getByTestId('prv-header-chip');
  await chip.click();
  await page.getByTestId('prv-settings-test').click();
  await expect(chip).toHaveAttribute('data-state', 'offline');
  await expect(chip).toHaveText('offline · unknown');
});

test('FR-PRV-07 a working connection shows ok and the balance', async ({ page }) => {
  await mockProvider(page, { ...base, stored: 'k-wxyz', testResult: 'ok' });
  const chip = page.getByTestId('prv-header-chip');
  await chip.click();
  await page.getByTestId('prv-settings-test').click();
  await expect(page.getByTestId('prv-settings-result')).toHaveText('Connected.');
  await expect(chip).toHaveText('Scenario · ok · 1,234 CU');
});
